/*!
# DAT2 Archive Format (Fallout 2)

Little-endian, flat file list, zlib compression, parallel extraction via rayon.

## File layout:
1. File data (all files concatenated)
2. Directory tree (file count + file entries)
3. Footer (8 bytes): tree_size + dat_size
*/

use anyhow::{Context, Result, bail};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use deku::prelude::*;
use std::io::{Cursor, Write};
use std::path::Path;

use crate::common::{self, CompressionLevel, ExtractionMode, FileEntry, utils};

/// 8-byte footer at the end of every DAT2 file.
/// Points to the directory tree and validates the total file size.
#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "little")]
struct Dat2Footer {
    tree_size: u32,
    dat_size: u32,
}

/// File entry as stored in the DAT2 directory tree
#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "little")]
struct Dat2FileEntry {
    filename_size: u32,
    #[deku(count = "filename_size")]
    filename_bytes: Vec<u8>,
    compression_type: u8, // 0 = uncompressed, 1 = zlib
    real_size: u32,
    packed_size: u32,
    offset: u32,
}

/// DAT2 archive handler (Fallout 2 format)
#[derive(Debug)]
pub struct Dat2Archive {
    files: Vec<FileEntry>,
    /// Raw archive data for reading existing file content
    data: Vec<u8>,
}

impl Dat2Archive {
    /// Create a new empty DAT2 archive
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            data: Vec::new(),
        }
    }

    /// Parse an existing DAT2 archive from raw bytes
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        if data.len() < 8 {
            bail!("DAT2 file too small");
        }

        let files = Self::parse_directory_tree(&data)?;
        Ok(Self { files, data })
    }

    fn parse_directory_tree(data: &[u8]) -> Result<Vec<FileEntry>> {
        // Parse 8-byte footer at end of file
        let footer_bytes = &data[data.len() - 8..];
        let (_, footer) = Dat2Footer::from_bytes((footer_bytes, 0))
            .map_err(|e| anyhow::anyhow!("Failed to parse DAT2 footer: {}", e))?;

        if footer.dat_size as usize != data.len() {
            bail!(
                "DAT size mismatch: expected {}, got {}",
                footer.dat_size,
                data.len()
            );
        }

        // Directory tree position: dat_size - tree_size - 8 (footer).
        // tree_size is untrusted archive input; checked math turns a hostile
        // value into a clean error instead of an underflow.
        let tree_start = (footer.dat_size as usize)
            .checked_sub(footer.tree_size as usize)
            .and_then(|v| v.checked_sub(8))
            .context("Invalid DAT2 footer: tree size exceeds file size")?;
        // The tree must at least hold its own 4-byte file count. tree_start itself may
        // legitimately be 0: an archive whose files are all empty has no data section.
        if footer.tree_size < 4 {
            bail!("Invalid DAT2 footer: directory tree too small");
        }

        // Read file count
        let mut cursor = Cursor::new(&data[tree_start..]);
        let file_count = cursor
            .read_u32::<LittleEndian>()
            .context("Failed to read file count from DAT2 directory tree")?;

        // Parse file entries using deku. No preallocation from file_count:
        // it is untrusted input and a crafted value could reserve gigabytes.
        let mut files = Vec::new();
        let tree_data = &data[tree_start + 4..data.len() - 8];
        let mut current_offset = 0;

        for i in 0..file_count {
            let remaining_data = &tree_data[current_offset..];
            let ((remaining_slice, _bit_offset), entry) =
                Dat2FileEntry::from_bytes((remaining_data, 0))
                    .map_err(|e| anyhow::anyhow!("Failed to parse file entry: {}", e))?;

            let filename = utils::decode_filename(&entry.filename_bytes)
                .with_context(|| format!("Failed to decode filename for file entry {i}"))?;

            files.push(FileEntry {
                name: filename,
                offset: entry.offset as u64,
                size: entry.real_size,
                packed_size: entry.packed_size,
                compressed: entry.compression_type == 1,
                data: None,
            });

            let bytes_consumed = remaining_data.len() - remaining_slice.len();
            current_offset += bytes_consumed;
        }

        Ok(files)
    }

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, files: &[String]) -> Result<()> {
        let all_files: Vec<&FileEntry> = self.files.iter().collect();
        common::list_files_filtered(&all_files, files)
    }

    /// Extract files from the archive using parallel processing
    pub fn extract(&self, output_dir: &Path, files: &[String], mode: ExtractionMode) -> Result<()> {
        let files_to_extract = common::filter_files_by_patterns(&self.files, files)?;
        common::extract_zlib_archive_parallel(&self.data, &files_to_extract, output_dir, mode)
    }

    /// Read file data from the archive's own data buffer
    fn read_file_data(&self, file: &FileEntry) -> Result<Vec<u8>> {
        utils::read_file_slice(&self.data, file)
    }

    /// Add files to the archive (directories processed recursively, parallel)
    pub fn add_file(
        &mut self,
        file_path: &Path,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        source_root: Option<&Path>,
    ) -> Result<()> {
        common::add_files_zlib(
            &mut self.files,
            file_path,
            compression,
            target_dir,
            source_root,
        )
    }

    /// Delete a file from the archive by name
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        common::delete_file_from_list(&mut self.files, file_name)
    }

    /// Save the archive to a DAT2 file.
    ///
    /// DAT2 layout: file data, then directory tree, then 8-byte footer.
    pub fn save(&self, path: &Path) -> Result<()> {
        // DAT2 stores file offsets as u32. Entries keep data.len() == packed_size,
        // so this bounds the u32 offset accumulation below.
        let total_payload: u64 = self.files.iter().map(|f| f.packed_size as u64).sum();
        if total_payload > u32::MAX as u64 {
            bail!("DAT2 archive would exceed the format's 4 GiB offset limit");
        }

        utils::write_atomically(path, |cursor| {
            // Step 1: Write all file data
            let mut current_offset = 0u32;
            let mut file_offsets = Vec::new();

            for file in &self.files {
                file_offsets.push(current_offset);

                let owned;
                let data: &[u8] = match file.data {
                    Some(ref file_data) => file_data, // Already in memory (newly added file)
                    None => {
                        owned = self.read_file_data(file)?; // Read from the original archive
                        &owned
                    }
                };

                cursor.write_all(data)?;
                current_offset += data.len() as u32;
            }

            // Step 2: Write directory tree, tracking its size since a file
            // writer has no cheap position() like the old in-memory cursor
            let tree_start = current_offset as u64;
            cursor.write_u32::<LittleEndian>(self.files.len() as u32)?;
            let mut tree_size: u64 = 4;

            for (i, file) in self.files.iter().enumerate() {
                let entry = Dat2FileEntry {
                    filename_size: file.name.len() as u32,
                    filename_bytes: file.name.as_bytes().to_vec(),
                    compression_type: if file.compressed { 1 } else { 0 },
                    real_size: file.size,
                    packed_size: file.packed_size,
                    offset: file_offsets[i],
                };

                let entry_bytes = entry.to_bytes()?;
                cursor.write_all(&entry_bytes)?;
                tree_size += entry_bytes.len() as u64;
            }

            // Step 3: Write 8-byte footer
            let total_size = tree_start + tree_size + 8;

            let footer = Dat2Footer {
                tree_size: tree_size as u32,
                dat_size: u32::try_from(total_size)
                    .context("DAT2 archive would exceed the format's 4 GiB size limit")?,
            };
            let footer_bytes = footer.to_bytes()?;
            cursor.write_all(&footer_bytes)?;

            Ok(())
        })
        .context("Failed to write DAT2 file")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn save_then_parse_round_trips(
            payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..512), 1..8)
        ) {
            let mut archive = Dat2Archive::new();
            for (i, data) in payloads.iter().enumerate() {
                let mut entry = FileEntry::with_data(format!("F{i}.BIN"), data.clone(), false);
                entry.size = data.len() as u32;
                archive.files.push(entry);
            }

            let target = std::env::temp_dir()
                .join(format!("dat3_prop_dat2_{}.dat", std::process::id()));
            archive.save(&target).unwrap();
            let bytes = std::fs::read(&target).unwrap();
            std::fs::remove_file(&target).ok();

            let reparsed = Dat2Archive::from_bytes(bytes).unwrap();
            prop_assert_eq!(reparsed.files.len(), payloads.len());
            for (i, data) in payloads.iter().enumerate() {
                prop_assert_eq!(&reparsed.files[i].name, &format!("F{i}.BIN"));
                prop_assert_eq!(reparsed.files[i].size as usize, data.len());
                prop_assert!(!reparsed.files[i].compressed);
                let read_back = reparsed.read_file_data(&reparsed.files[i]).unwrap();
                prop_assert_eq!(&read_back, data);
            }
        }

        #[test]
        fn from_bytes_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
            let _ = Dat2Archive::from_bytes(bytes);
        }
    }

    #[test]
    fn extract_errors_and_writes_nothing_when_a_requested_file_is_missing() {
        let mut archive = Dat2Archive::new();
        let mut entry = FileEntry::with_data("A.TXT".to_string(), b"data".to_vec(), false);
        entry.size = 4;
        archive.files.push(entry);

        let dir =
            std::env::temp_dir().join(format!("dat3_dat2_extract_missing_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("t.dat");
        archive.save(&target).unwrap();
        let reparsed = Dat2Archive::from_bytes(std::fs::read(&target).unwrap()).unwrap();

        let out = dir.join("out");
        let patterns = vec!["A.TXT".to_string(), "NOPE.TXT".to_string()];
        let err = reparsed
            .extract(&out, &patterns, ExtractionMode::PreserveStructure)
            .unwrap_err();

        assert!(
            err.to_string().contains("not found"),
            "unexpected error: {err}"
        );
        // Missing patterns are rejected before anything is written, so a typo never
        // leaves a half-populated output directory.
        assert!(!out.join("A.TXT").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn round_trips_archive_whose_only_file_is_empty() {
        // Zero total data bytes puts the directory tree at offset 0; the parser must accept it.
        let mut archive = Dat2Archive::new();
        archive.files.push(FileEntry::with_data(
            "EMPTY.TXT".to_string(),
            Vec::new(),
            false,
        ));

        let target =
            std::env::temp_dir().join(format!("dat3_dat2_empty_{}.dat", std::process::id()));
        archive.save(&target).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        std::fs::remove_file(&target).ok();

        let reparsed = Dat2Archive::from_bytes(bytes).unwrap();
        assert_eq!(reparsed.files.len(), 1);
        assert_eq!(reparsed.files[0].name, "EMPTY.TXT");
        assert_eq!(reparsed.files[0].size, 0);
    }

    #[test]
    fn from_bytes_errors_when_tree_size_exceeds_file_size() {
        // 12-byte file whose footer claims a tree larger than the whole file:
        // must produce a clean error, not an arithmetic underflow.
        let mut data = vec![0u8; 4];
        data.extend_from_slice(&0xFFFF_FFF0u32.to_le_bytes()); // tree_size
        data.extend_from_slice(&12u32.to_le_bytes()); // dat_size == file length
        assert!(Dat2Archive::from_bytes(data).is_err());
    }

    #[test]
    fn save_errors_when_payload_exceeds_u32_offsets() {
        let huge_entry = |name: &str| FileEntry {
            name: name.to_string(),
            offset: 0,
            size: u32::MAX,
            packed_size: u32::MAX,
            compressed: false,
            data: Some(Vec::new()),
        };
        let archive = Dat2Archive {
            files: vec![huge_entry("A.TXT"), huge_entry("B.TXT")],
            data: Vec::new(),
        };
        let target = std::env::temp_dir().join("dat3_dat2_overflow_test.dat");
        assert!(archive.save(&target).is_err());
    }
}
