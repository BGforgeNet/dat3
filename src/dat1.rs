/*!
# DAT1 Archive Format (Fallout 1)

Big-endian, hierarchical directory structure, LZSS compression.

## File layout:
1. Header (16 bytes): directory count + 3 unknown fields
2. Directory names: length byte + name for each directory
3. Directory contents: header + file entries per directory
4. File data: raw content, stored in order

LZSS compression for writing is not implemented - files are stored uncompressed.
*/

use anyhow::{Context, Result, bail};
use deku::prelude::*;
use rayon::prelude::*;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::common::{self, CompressionLevel, ExtractionMode, FileEntry, ListFormat, utils};
use crate::lzss;

// DAT1 format constants
const DAT1_COMPRESSED_FLAG: u32 = 0x40;
const DAT1_UNCOMPRESSED_FLAG: u32 = 0x20;
const DAT1_FORMAT_ID: u32 = 0x0A;
const DAT1_DIRECTORY_UNKNOWN5: u32 = 0x10;

/// Serialized sizes of the fixed-layout headers, derived from their structs
const HEADER_SIZE: u32 = Dat1Header::SIZE_BYTES.unwrap() as u32;
const DIR_HEADER_SIZE: u32 = Dat1DirHeader::SIZE_BYTES.unwrap() as u32;

/// 16-byte archive header
#[derive(Debug, DekuRead, DekuWrite, DekuSize)]
#[deku(endian = "big")]
struct Dat1Header {
    dir_count: u32,
    format_id: u32, // format identifier seen in original files
    unknown2: u32,  // always zero in practice
    unknown3: u32,  // always zero in practice
}

/// Length-prefixed name, used for directory names
#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "big")]
struct Dat1Name {
    len: u8,
    #[deku(count = "len")]
    bytes: Vec<u8>,
}

/// 16-byte per-directory content header
#[derive(Debug, DekuRead, DekuWrite, DekuSize)]
#[deku(endian = "big")]
struct Dat1DirHeader {
    file_count: u32,
    unknown4: u32,
    unknown5: u32,
    unknown6: u32,
}

/// File entry as stored in a directory's content block
#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "big")]
struct Dat1FileEntry {
    name_len: u8,
    #[deku(count = "name_len")]
    name_bytes: Vec<u8>,
    attributes: u32,
    offset: u32,
    size: u32,
    packed_size: u32,
}

/// A directory within a DAT1 archive.
/// DAT1 uses hierarchical directories; the root is named ".".
#[derive(Debug, Clone)]
struct Directory {
    name: String,
    files: Vec<FileEntry>,
}

/// DAT1 archive handler (Fallout 1 format)
#[derive(Debug)]
pub struct Dat1Archive {
    directories: Vec<Directory>,
    /// Raw archive data for reading existing file content
    data: Vec<u8>,
}

impl Dat1Archive {
    /// Create a new empty DAT1 archive with just a root directory
    pub fn new() -> Self {
        Self {
            directories: vec![Directory {
                name: ".".to_string(), // "." is the root directory in DAT1 format
                files: Vec::new(),
            }],
            data: Vec::new(),
        }
    }

    /// Parse an existing DAT1 archive from raw bytes
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let ((mut rest, _), header) = Dat1Header::from_bytes((&data, 0))
            .map_err(|e| anyhow::anyhow!("Failed to parse DAT1 header: {e}"))?;

        // Read directory names
        let mut dir_names = Vec::new();
        for i in 0..header.dir_count {
            let ((r, _), name) = Dat1Name::from_bytes((rest, 0))
                .map_err(|e| anyhow::anyhow!("Failed to parse name for directory {i}: {e}"))?;
            rest = r;
            dir_names.push(
                utils::decode_filename(&name.bytes).context("Failed to decode directory name")?,
            );
        }

        // Read directory contents (file entries per directory)
        let mut directories = Vec::new();
        for dir_name in dir_names {
            let ((r, _), dir_header) = Dat1DirHeader::from_bytes((rest, 0)).map_err(|e| {
                anyhow::anyhow!("Failed to parse content header for directory '{dir_name}': {e}")
            })?;
            rest = r;

            let mut files = Vec::new();
            for j in 0..dir_header.file_count {
                let ((r, _), entry) = Dat1FileEntry::from_bytes((rest, 0)).map_err(|e| {
                    anyhow::anyhow!("Failed to parse file entry {j} in directory '{dir_name}': {e}")
                })?;
                rest = r;

                let name = utils::decode_filename(&entry.name_bytes)
                    .context("Failed to decode file name")?;
                let compressed = entry.attributes & DAT1_COMPRESSED_FLAG != 0;
                let actual_packed_size = if entry.packed_size == 0 {
                    entry.size
                } else {
                    entry.packed_size
                };
                let full_name = if dir_name == "." {
                    name
                } else {
                    format!("{dir_name}\\{name}")
                };

                files.push(FileEntry {
                    name: full_name,
                    offset: entry.offset as u64,
                    size: entry.size,
                    packed_size: actual_packed_size,
                    compressed,
                    data: None,
                });
            }

            directories.push(Directory {
                name: dir_name,
                files,
            });
        }

        Ok(Self { directories, data })
    }

    /// Collect references to all files across all directories
    fn all_files(&self) -> Vec<&FileEntry> {
        self.directories.iter().flat_map(|dir| &dir.files).collect()
    }

    /// Collect all files as a flat owned list (for filter_files_by_patterns)
    fn all_files_flat(&self) -> Vec<FileEntry> {
        self.directories
            .iter()
            .flat_map(|dir| dir.files.clone())
            .collect()
    }

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, files: &[String], format: ListFormat) -> Result<()> {
        let all_files = self.all_files();
        common::list_files_filtered(&all_files, files, format)
    }

    /// Extract files from the archive in parallel, mirroring the DAT2 path
    /// (per-file LZSS decompression and disk writes are independent).
    pub fn extract(&self, output_dir: &Path, files: &[String], mode: ExtractionMode) -> Result<()> {
        let all_flat = self.all_files_flat();
        let files_to_extract = common::filter_files_by_patterns(&all_flat, files)?;
        let total_files = files_to_extract.len();
        let completed = AtomicUsize::new(0);

        println!("Extracting {total_files} files...");
        let start = Instant::now();

        files_to_extract
            .par_iter()
            .try_for_each(|file| -> Result<()> {
                utils::validate_archive_path(&file.name)?;

                // Progress reporting every 1000 files
                let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if count.is_multiple_of(1000) || count == total_files {
                    let elapsed = start.elapsed().as_millis();
                    let files_per_sec = count as f64 / elapsed as f64 * 1000.0;
                    println!(
                        "Progress: {count}/{total_files} files extracted ({files_per_sec:.1} files/sec)"
                    );
                }

                let output_path = utils::resolve_output_path(output_dir, &file.name, mode);

                utils::ensure_dir_exists(&output_path)?;

                let file_data = self
                    .read_file_data(file)
                    .with_context(|| format!("Failed to read data for file '{}'", file.name))?;

                // Decompress LZSS if needed
                let final_data = if file.compressed {
                    lzss::decompress(&file_data, file.size as usize)
                        .with_context(|| format!("Failed to decompress {}", file.name))?
                } else {
                    file_data
                };

                fs::write(&output_path, final_data)
                    .with_context(|| format!("Failed to write {}", output_path.display()))?;

                Ok(())
            })?;

        println!(
            "Extraction completed in {:.2}s",
            start.elapsed().as_secs_f64()
        );
        Ok(())
    }

    /// Read file data from the raw archive bytes
    fn read_file_data(&self, file: &FileEntry) -> Result<Vec<u8>> {
        utils::read_file_slice(&self.data, file)
    }

    /// Add files to the archive.
    /// DAT1 compression (LZSS) is not implemented - files are stored uncompressed.
    pub fn add_file(
        &mut self,
        file_path: &Path,
        _compression: CompressionLevel,
        target_dir: Option<&str>,
        source_root: Option<&Path>,
    ) -> Result<()> {
        let base_path = file_path;
        let files = utils::collect_files(file_path).with_context(|| {
            format!(
                "Failed to collect files from path '{}'",
                file_path.display()
            )
        })?;

        for file in files {
            let data =
                fs::read(&file).with_context(|| format!("Failed to read {}", file.display()))?;

            let archive_path =
                utils::calculate_archive_path(&file, base_path, target_dir, source_root)?;

            let size = data.len() as u32;
            let display_path = utils::normalize_path_for_display(&archive_path);
            println!("Adding: {display_path}");

            // Find or create target directory
            let dir_name = utils::get_dirname_from_dat_path(&archive_path);
            let dir_index =
                if let Some(index) = self.directories.iter().position(|d| d.name == dir_name) {
                    index
                } else {
                    self.directories.push(Directory {
                        name: dir_name.to_string(),
                        files: Vec::new(),
                    });
                    self.directories.len() - 1
                };

            // Remove any existing file with the same name from all directories
            for dir in &mut self.directories {
                dir.files
                    .retain(|existing_file| existing_file.name != archive_path);
            }

            // DAT1 stores files uncompressed
            let mut file_entry = FileEntry::with_data(archive_path, data, false);
            file_entry.size = size;
            self.directories[dir_index].files.push(file_entry);
        }

        Ok(())
    }

    /// Delete a file from the archive by name
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        let normalized_name = utils::normalize_user_path(file_name).into_owned();

        for dir in &mut self.directories {
            if let Some(pos) = dir.files.iter().position(|f| f.name == normalized_name) {
                let display_name = utils::normalize_path_for_display(&normalized_name);
                println!("Deleting: {display_name}");
                dir.files.remove(pos);
                return Ok(());
            }
        }

        bail!(
            "File not found: {}",
            utils::normalize_path_for_display(file_name)
        );
    }

    /// Save the archive to a file
    pub fn save(&self, path: &Path) -> Result<()> {
        // Calculate where file data starts: header, directory names, then
        // directory content blocks. Computed up front so entry offsets are
        // known before anything is written.
        let mut data_offset: u32 = HEADER_SIZE;
        for dir in &self.directories {
            data_offset += 1 + dir.name.len() as u32; // Length-prefixed directory name
        }
        for dir in &self.directories {
            data_offset += DIR_HEADER_SIZE;
            for file in &dir.files {
                // Not derivable: the length-prefixed name makes `Dat1FileEntry` variable-size.
                // name_len byte + stored name + 4 u32 entry fields
                data_offset += 1 + stored_file_name(&dir.name, &file.name).len() as u32 + 16;
            }
        }

        // DAT1 stores file offsets as u32; reject payloads the format cannot address.
        let total_payload: u64 = self
            .directories
            .iter()
            .flat_map(|d| &d.files)
            .map(|f| f.packed_size as u64)
            .sum();
        if data_offset as u64 + total_payload > u32::MAX as u64 {
            bail!("DAT1 archive would exceed the format's 4 GiB offset limit");
        }

        utils::write_atomically(path, |output| {
            output.write_all(
                &Dat1Header {
                    dir_count: self.directories.len() as u32,
                    format_id: DAT1_FORMAT_ID,
                    unknown2: 0,
                    unknown3: 0,
                }
                .to_bytes()?,
            )?;

            // Write directory names
            for dir in &self.directories {
                output.write_all(
                    &Dat1Name {
                        len: dir.name.len() as u8,
                        bytes: dir.name.as_bytes().to_vec(),
                    }
                    .to_bytes()?,
                )?;
            }

            let mut current_offset = data_offset;

            // Write directory content headers and file entries
            for dir in &self.directories {
                output.write_all(
                    &Dat1DirHeader {
                        file_count: dir.files.len() as u32,
                        unknown4: DAT1_FORMAT_ID,
                        unknown5: DAT1_DIRECTORY_UNKNOWN5,
                        unknown6: 0,
                    }
                    .to_bytes()?,
                )?;

                for file in &dir.files {
                    let stored_name = stored_file_name(&dir.name, &file.name);
                    let entry = Dat1FileEntry {
                        name_len: stored_name.len() as u8,
                        name_bytes: stored_name.as_bytes().to_vec(),
                        attributes: if file.compressed {
                            DAT1_COMPRESSED_FLAG
                        } else {
                            DAT1_UNCOMPRESSED_FLAG
                        },
                        offset: current_offset,
                        size: file.size,
                        packed_size: if file.compressed { file.packed_size } else { 0 },
                    };
                    output.write_all(&entry.to_bytes()?)?;

                    current_offset += file.packed_size;
                }
            }

            // Write file data, borrowing in-memory entries instead of cloning
            for dir in &self.directories {
                for file in &dir.files {
                    match file.data {
                        Some(ref file_data) => output.write_all(file_data)?,
                        None => output.write_all(&self.read_file_data(file)?)?,
                    }
                }
            }

            Ok(())
        })
        .context("Failed to write DAT1 file")?;

        Ok(())
    }
}

/// Name as stored in a directory's content block: the directory prefix is
/// stripped for real directories; root (".") entries are stored as-is.
fn stored_file_name<'a>(dir_name: &str, file_name: &'a str) -> &'a str {
    if dir_name == "." {
        return file_name;
    }
    file_name
        .strip_prefix(dir_name)
        .and_then(|rest| rest.strip_prefix('\\'))
        .unwrap_or(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The derived sizes are part of the on-disk format: a field added to
    /// either header would silently move every file offset.
    #[test]
    fn header_sizes_match_the_on_disk_format() {
        assert_eq!(HEADER_SIZE, 16);
        assert_eq!(DIR_HEADER_SIZE, 16);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn save_then_parse_round_trips(
            payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..512), 1..8)
        ) {
            let mut archive = Dat1Archive::new();
            for (i, data) in payloads.iter().enumerate() {
                let mut entry = FileEntry::with_data(format!("F{i}.BIN"), data.clone(), false);
                entry.size = data.len() as u32;
                archive.directories[0].files.push(entry);
            }

            let target = std::env::temp_dir()
                .join(format!("dat3_prop_dat1_{}.dat", std::process::id()));
            archive.save(&target).unwrap();
            let bytes = std::fs::read(&target).unwrap();
            std::fs::remove_file(&target).ok();

            let reparsed = Dat1Archive::from_bytes(bytes).unwrap();
            prop_assert_eq!(reparsed.directories.len(), 1);
            let files = &reparsed.directories[0].files;
            prop_assert_eq!(files.len(), payloads.len());
            for (i, data) in payloads.iter().enumerate() {
                prop_assert_eq!(&files[i].name, &format!("F{i}.BIN"));
                prop_assert_eq!(files[i].size as usize, data.len());
                let read_back = reparsed.read_file_data(&files[i]).unwrap();
                prop_assert_eq!(&read_back, data);
            }
        }

        #[test]
        fn from_bytes_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
            let _ = Dat1Archive::from_bytes(bytes);
        }
    }

    #[test]
    fn extract_writes_decompressed_entries_to_disk() {
        // Valid literal-only LZSS stream for b"HELLO": one compressed block of
        // 6 bytes - flag 0xFF (all-literal), then the 5 literals.
        let lzss_stream = vec![0x00, 0x06, 0xFF, b'H', b'E', b'L', b'L', b'O'];

        let mut archive = Dat1Archive::new();
        let mut plain =
            FileEntry::with_data("PLAIN.TXT".to_string(), b"plain data".to_vec(), false);
        plain.size = 10; // with_data leaves size unset; DAT1 reads uncompressed lengths from it
        archive.directories[0].files.push(plain);
        let mut packed = FileEntry::with_data("PACKED.TXT".to_string(), lzss_stream, true);
        packed.size = 5; // decompressed length of "HELLO"
        archive.directories[0].files.push(packed);

        let dir = std::env::temp_dir().join(format!("dat3_dat1_extract_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("t.dat");
        archive.save(&target).unwrap();

        let reparsed = Dat1Archive::from_bytes(std::fs::read(&target).unwrap()).unwrap();
        let out = dir.join("out");
        reparsed
            .extract(&out, &[], ExtractionMode::PreserveStructure)
            .unwrap();

        assert_eq!(std::fs::read(out.join("PLAIN.TXT")).unwrap(), b"plain data");
        assert_eq!(std::fs::read(out.join("PACKED.TXT")).unwrap(), b"HELLO");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn extract_errors_and_writes_nothing_when_a_requested_file_is_missing() {
        let mut archive = Dat1Archive::new();
        let mut plain =
            FileEntry::with_data("PLAIN.TXT".to_string(), b"plain data".to_vec(), false);
        plain.size = 10;
        archive.directories[0].files.push(plain);

        let dir =
            std::env::temp_dir().join(format!("dat3_dat1_extract_missing_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("t.dat");
        archive.save(&target).unwrap();
        let reparsed = Dat1Archive::from_bytes(std::fs::read(&target).unwrap()).unwrap();

        let out = dir.join("out");
        let patterns = vec!["PLAIN.TXT".to_string(), "NOPE.TXT".to_string()];
        let err = reparsed
            .extract(&out, &patterns, ExtractionMode::PreserveStructure)
            .unwrap_err();

        assert!(
            err.to_string().contains("not found"),
            "unexpected error: {err}"
        );
        assert!(!out.join("PLAIN.TXT").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_errors_when_payload_exceeds_u32_offsets() {
        let archive = Dat1Archive {
            directories: vec![Directory {
                name: ".".to_string(),
                files: vec![FileEntry {
                    name: "A.TXT".to_string(),
                    offset: 0,
                    size: u32::MAX,
                    packed_size: u32::MAX,
                    compressed: false,
                    data: Some(Vec::new()),
                }],
            }],
            data: Vec::new(),
        };
        let target = std::env::temp_dir().join("dat3_dat1_overflow_test.dat");
        assert!(archive.save(&target).is_err());
    }
}
