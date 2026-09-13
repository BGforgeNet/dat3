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
use std::borrow::Cow;
use std::io::{Cursor, Write};
use std::path::Path;

use crate::common::{
    self, CaseMode, CompressionLevel, ExtractionMode, FileEntry, ListFormat, MAX_PATH_BYTES,
    Selection, utils,
};

/// 8-byte footer at the end of every DAT2 file.
/// Points to the directory tree and validates the total file size.
#[derive(Debug, DekuRead, DekuWrite, DekuSize)]
#[deku(endian = "little")]
struct Dat2Footer {
    tree_size: u32,
    dat_size: u32,
}

/// Size of the trailing footer in bytes, derived from `Dat2Footer`
const FOOTER_SIZE: usize = Dat2Footer::SIZE_BYTES.unwrap();

/// File entry as stored in the DAT2 directory tree
#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "little")]
struct Dat2FileEntry {
    // Checked as soon as it is read: deku sizes the name buffer from `count`
    // up front, so an unchecked crafted length is an allocation of up to 4 GiB.
    #[deku(assert = "*filename_size as usize <= MAX_PATH_BYTES")]
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

impl Default for Dat2Archive {
    fn default() -> Self {
        Self::new()
    }
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
        if data.len() < FOOTER_SIZE {
            bail!("DAT2 file too small");
        }

        let files = Self::parse_directory_tree(&data)?;
        Ok(Self { files, data })
    }

    fn parse_directory_tree(data: &[u8]) -> Result<Vec<FileEntry>> {
        // Parse the footer at end of file
        let footer_bytes = &data[data.len() - FOOTER_SIZE..];
        let (_, footer) = Dat2Footer::from_bytes((footer_bytes, 0))
            .map_err(|e| common::deku_parse_error("Failed to parse DAT2 footer", e))?;

        if footer.dat_size as usize != data.len() {
            bail!(
                "DAT size mismatch: expected {}, got {}",
                footer.dat_size,
                data.len()
            );
        }

        // Directory tree position: dat_size - tree_size - footer.
        // tree_size is untrusted archive input; checked math turns a hostile
        // value into a clean error instead of an underflow.
        let tree_start = (footer.dat_size as usize)
            .checked_sub(footer.tree_size as usize)
            .and_then(|v| v.checked_sub(FOOTER_SIZE))
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
        let tree_data = &data[tree_start + 4..data.len() - FOOTER_SIZE];
        let mut current_offset = 0;

        for i in 0..file_count {
            let remaining_data = &tree_data[current_offset..];
            let ((remaining_slice, _bit_offset), entry) =
                Dat2FileEntry::from_bytes((remaining_data, 0)).map_err(|e| {
                    common::deku_parse_error(format_args!("Failed to parse file entry {i}"), e)
                })?;

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

    /// References to every file entry
    pub fn entries(&self) -> Vec<&FileEntry> {
        self.files.iter().collect()
    }

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, selection: &Selection, format: ListFormat) -> Result<()> {
        common::list_files_filtered(&self.entries(), selection, format)
    }

    /// Extract files from the archive using parallel processing
    pub fn extract(
        &self,
        output_dir: &Path,
        mode: ExtractionMode,
        selection: &Selection,
    ) -> Result<()> {
        common::extract_matching(
            &self.data,
            &self.files,
            output_dir,
            mode,
            selection,
            common::decompress_zlib,
        )
    }

    /// Read file data from the archive's own data buffer
    fn read_file_data<'a>(&'a self, file: &'a FileEntry) -> Result<&'a [u8]> {
        utils::read_file_slice(&self.data, file)
    }

    /// Add files to the archive (directories processed recursively, parallel)
    pub fn add_file(
        &mut self,
        file_path: &Path,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        source_root: Option<&Path>,
        case: CaseMode,
    ) -> Result<()> {
        common::add_files_zlib(
            &mut self.files,
            file_path,
            compression,
            target_dir,
            source_root,
            case,
        )
    }

    /// An entry's contents, decompressed
    pub fn contents<'a>(&'a self, file: &'a FileEntry) -> Result<Cow<'a, [u8]>> {
        common::entry_contents(&self.data, file, common::decompress_zlib)
    }

    /// Add a prepared entry, replacing the entries of its name as `case` compares
    pub fn insert_entry(&mut self, entry: FileEntry, case: CaseMode) {
        common::merge_entries(&mut self.files, vec![entry], case);
    }

    /// Remove the entry named exactly `file_name`, reporting whether there was one
    pub fn remove_entry(&mut self, file_name: &str) -> bool {
        common::remove_from_list(&mut self.files, file_name)
    }

    /// Save the archive to a DAT2 file
    pub fn save(&self, path: &Path) -> Result<()> {
        self.prepare_save()?;
        utils::write_atomically(path, |out| self.write_prepared(out)).context(WRITE_CONTEXT)
    }

    /// Write the archive as a DAT2 file to `out`
    pub fn write_to(&self, out: &mut dyn Write) -> Result<()> {
        self.prepare_save()?;
        self.write_prepared(out).context(WRITE_CONTEXT)
    }

    /// Checks that fail before any output exists, so a refused save leaves no temp file
    fn prepare_save(&self) -> Result<()> {
        // DAT2 stores file offsets as u32. Entries keep data.len() == packed_size,
        // so this bounds the u32 offset accumulation below.
        let total_payload: u64 = self.files.iter().map(|f| f.packed_size as u64).sum();
        if total_payload > u32::MAX as u64 {
            bail!("DAT2 archive would exceed the format's 4 GiB offset limit");
        }
        Ok(())
    }

    /// DAT2 layout: file data, then directory tree, then 8-byte footer.
    fn write_prepared(&self, cursor: &mut dyn Write) -> Result<()> {
        // Step 1: Write all file data
        let mut current_offset = 0u32;
        let mut file_offsets = Vec::new();

        for file in &self.files {
            file_offsets.push(current_offset);

            // In memory for a newly added file, borrowed from the original archive otherwise
            let data = self.read_file_data(file)?;

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

        // Step 3: Write the footer
        let total_size = tree_start + tree_size + FOOTER_SIZE as u64;

        let footer = Dat2Footer {
            tree_size: tree_size as u32,
            dat_size: u32::try_from(total_size)
                .context("DAT2 archive would exceed the format's 4 GiB size limit")?,
        };
        let footer_bytes = footer.to_bytes()?;
        cursor.write_all(&footer_bytes)?;

        Ok(())
    }
}

const WRITE_CONTEXT: &str = "Failed to write DAT2 file";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::MissingFiles;
    use crate::test_support::ScratchPath;
    use proptest::prelude::*;

    /// The derived size is part of the on-disk format: a field added to
    /// `Dat2Footer` would silently move the directory tree.
    #[test]
    fn footer_size_matches_the_on_disk_format() {
        assert_eq!(FOOTER_SIZE, 8);
    }

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

            let target = ScratchPath::new("prop_dat2");
            archive.save(&target).unwrap();
            let bytes = std::fs::read(&target).unwrap();

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

        let dir = ScratchPath::new("dat2_extract_missing");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("t.dat");
        archive.save(&target).unwrap();
        let reparsed = Dat2Archive::from_bytes(std::fs::read(&target).unwrap()).unwrap();

        let out = dir.join("out");
        let patterns = vec!["A.TXT".to_string(), "NOPE.TXT".to_string()];
        let err = reparsed
            .extract(
                &out,
                ExtractionMode::PreserveStructure,
                &crate::test_support::exact(&patterns, MissingFiles::Fail),
            )
            .unwrap_err();

        assert!(
            err.to_string().contains("not found"),
            "unexpected error: {err}"
        );
        // Missing patterns are rejected before anything is written, so a typo never
        // leaves a half-populated output directory.
        assert!(!out.join("A.TXT").exists());
    }

    #[test]
    fn extract_ignoring_missing_writes_the_files_that_are_present() {
        let mut archive = Dat2Archive::new();
        let mut entry = FileEntry::with_data("A.TXT".to_string(), b"data".to_vec(), false);
        entry.size = 4;
        archive.files.push(entry);

        let dir = ScratchPath::new("dat2_extract_ignoring_missing");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("t.dat");
        archive.save(&target).unwrap();
        let reparsed = Dat2Archive::from_bytes(std::fs::read(&target).unwrap()).unwrap();

        let out = dir.join("out");
        let patterns = vec!["A.TXT".to_string(), "NOPE.TXT".to_string()];
        reparsed
            .extract(
                &out,
                ExtractionMode::PreserveStructure,
                &crate::test_support::exact(&patterns, MissingFiles::Warn),
            )
            .unwrap();

        assert!(out.join("A.TXT").exists());
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

        let target = ScratchPath::new("dat2_empty");
        archive.save(&target).unwrap();
        let bytes = std::fs::read(&target).unwrap();

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
        let target = ScratchPath::new("dat2_overflow");
        assert!(archive.save(&target).is_err());
    }

    /// One-entry archive storing `data` uncompressed under the raw name bytes
    fn single_entry_archive(name: &[u8], data: &[u8]) -> Vec<u8> {
        let mut tree = 1u32.to_le_bytes().to_vec();
        tree.extend_from_slice(&(name.len() as u32).to_le_bytes());
        tree.extend_from_slice(name);
        tree.push(0); // compression_type: uncompressed
        tree.extend_from_slice(&(data.len() as u32).to_le_bytes()); // real_size
        tree.extend_from_slice(&(data.len() as u32).to_le_bytes()); // packed_size
        tree.extend_from_slice(&0u32.to_le_bytes()); // offset

        let mut out = data.to_vec();
        out.extend_from_slice(&tree);
        let dat_size = out.len() + FOOTER_SIZE;
        out.extend_from_slice(&(tree.len() as u32).to_le_bytes());
        out.extend_from_slice(&(dat_size as u32).to_le_bytes());
        out
    }

    #[test]
    fn accepts_a_name_at_the_path_length_limit() {
        let name = vec![b'A'; MAX_PATH_BYTES];
        let parsed = Dat2Archive::from_bytes(single_entry_archive(&name, b"x")).unwrap();
        assert_eq!(parsed.files[0].name.len(), MAX_PATH_BYTES);
    }

    #[test]
    fn rejects_a_name_over_the_path_length_limit() {
        let name = vec![b'A'; MAX_PATH_BYTES + 1];
        let err = Dat2Archive::from_bytes(single_entry_archive(&name, b"x")).unwrap_err();
        assert!(
            err.to_string().contains("filename_size"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn rejects_a_hostile_name_length_before_reading_the_name() {
        // A 4 GiB length with no name behind it must fail on the length field,
        // not by sizing a buffer for the name.
        let mut archive = single_entry_archive(b"", b"");
        archive[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        let err = Dat2Archive::from_bytes(archive).unwrap_err();
        assert!(
            err.to_string().contains("filename_size"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn add_rejects_a_file_too_large_for_u32_sizes_without_reading_it() {
        // Sparse, so the test costs no disk and the check must come from metadata:
        // reading it would allocate 4 GiB.
        let dir = ScratchPath::dir("dat2_huge_file");
        let file = dir.join("HUGE.BIN");
        std::fs::File::create(&file)
            .unwrap()
            .set_len(u64::from(u32::MAX) + 1)
            .unwrap();

        let mut archive = Dat2Archive::new();
        let err = archive
            .add_file(
                &file,
                CompressionLevel::new(0).unwrap(),
                None,
                None,
                crate::common::CaseMode::Sensitive,
            )
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("larger than the 4 GiB a DAT archive entry can hold"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn add_rejects_a_path_over_the_length_limit() {
        let dir = ScratchPath::dir("dat2_long_path");
        let file = dir.join("F.TXT");
        std::fs::write(&file, b"x").unwrap();

        let mut archive = Dat2Archive::new();
        let target_dir = "D".repeat(MAX_PATH_BYTES);
        let err = archive
            .add_file(
                &file,
                CompressionLevel::new(0).unwrap(),
                Some(&target_dir),
                None,
                crate::common::CaseMode::Sensitive,
            )
            .unwrap_err();
        assert!(
            format!("{err:#}").contains(&format!("longer than {MAX_PATH_BYTES} bytes")),
            "unexpected error: {err:#}"
        );
    }
}
