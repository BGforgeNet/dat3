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

use anyhow::{bail, Context, Result};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use crate::common::{self, utils, CompressionLevel, ExtractionMode, FileEntry};
use crate::lzss;

// DAT1 format constants
const DAT1_COMPRESSED_FLAG: u32 = 0x40;
const DAT1_UNCOMPRESSED_FLAG: u32 = 0x20;
const DAT1_FORMAT_ID: u32 = 0x0A;
const DAT1_DIRECTORY_UNKNOWN5: u32 = 0x10;

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
        let mut cursor = Cursor::new(&data);

        // Read 16-byte header
        let dir_count = cursor
            .read_u32::<BigEndian>()
            .context("Failed to read directory count from DAT1 header")?;
        let _unknown1 = cursor
            .read_u32::<BigEndian>()
            .context("Failed to read unknown1 field from DAT1 header")?;
        let _unknown2 = cursor
            .read_u32::<BigEndian>()
            .context("Failed to read unknown2 field from DAT1 header")?;
        let _unknown3 = cursor
            .read_u32::<BigEndian>()
            .context("Failed to read unknown3 field from DAT1 header")?;

        // Read directory names
        let mut dir_names = Vec::new();
        for i in 0..dir_count {
            let name_len = cursor
                .read_u8()
                .with_context(|| format!("Failed to read name length for directory {i}"))?
                as usize;
            let mut name_bytes = vec![0u8; name_len];
            cursor
                .read_exact(&mut name_bytes)
                .with_context(|| format!("Failed to read name bytes for directory {i}"))?;
            let name =
                utils::decode_filename(&name_bytes).context("Failed to decode directory name")?;
            dir_names.push(name);
        }

        // Read directory contents (file entries per directory)
        let mut directories = Vec::new();
        for dir_name in dir_names {
            let file_count = cursor
                .read_u32::<BigEndian>()
                .with_context(|| format!("Failed to read file count for directory '{dir_name}'"))?;
            let _unknown4 = cursor.read_u32::<BigEndian>().with_context(|| {
                format!("Failed to read unknown4 field for directory '{dir_name}'")
            })?;
            let _unknown5 = cursor.read_u32::<BigEndian>().with_context(|| {
                format!("Failed to read unknown5 field for directory '{dir_name}'")
            })?;
            let _unknown6 = cursor.read_u32::<BigEndian>().with_context(|| {
                format!("Failed to read unknown6 field for directory '{dir_name}'")
            })?;

            let mut files = Vec::new();

            for j in 0..file_count {
                let name_len = cursor.read_u8().with_context(|| {
                    format!("Failed to read name length for file {j} in directory '{dir_name}'")
                })? as usize;
                let mut name_bytes = vec![0u8; name_len];
                cursor.read_exact(&mut name_bytes).with_context(|| {
                    format!("Failed to read name bytes for file {j} in directory '{dir_name}'")
                })?;
                let name =
                    utils::decode_filename(&name_bytes).context("Failed to decode file name")?;

                let attributes = cursor.read_u32::<BigEndian>().with_context(|| {
                    format!("Failed to read attributes for file '{name}' in directory '{dir_name}'")
                })?;
                let offset = cursor.read_u32::<BigEndian>().with_context(|| {
                    format!("Failed to read offset for file '{name}' in directory '{dir_name}'")
                })? as u64;
                let size = cursor.read_u32::<BigEndian>().with_context(|| {
                    format!("Failed to read size for file '{name}' in directory '{dir_name}'")
                })?;
                let packed_size = cursor.read_u32::<BigEndian>().with_context(|| {
                    format!(
                        "Failed to read packed size for file '{name}' in directory '{dir_name}'"
                    )
                })?;

                let compressed = attributes & DAT1_COMPRESSED_FLAG != 0;
                let actual_packed_size = if packed_size == 0 { size } else { packed_size };

                let full_name = if dir_name == "." {
                    name
                } else {
                    format!("{dir_name}\\{name}")
                };

                files.push(FileEntry {
                    name: full_name,
                    offset,
                    size,
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
    pub fn list(&self, files: &[String]) -> Result<()> {
        let all_files = self.all_files();
        common::list_files_filtered(&all_files, files)
    }

    /// Extract files from the archive
    pub fn extract(&self, output_dir: &Path, files: &[String], mode: ExtractionMode) -> Result<()> {
        let all_flat = self.all_files_flat();
        let files_to_extract = common::filter_files_by_patterns(&all_flat, files);

        for file in files_to_extract {
            utils::validate_archive_path(&file.name)?;

            let display_name = utils::normalize_path_for_display(&file.name);
            println!("Extracting: {display_name}");

            let output_path = match mode {
                ExtractionMode::Flat => {
                    let filename = utils::get_filename_from_dat_path(&file.name);
                    output_dir.join(filename)
                }
                ExtractionMode::PreserveStructure => {
                    output_dir.join(utils::to_system_path(&file.name))
                }
            };

            utils::ensure_dir_exists(&output_path)?;

            let file_data = self
                .read_file_data(file)
                .with_context(|| format!("Failed to read data for file '{}'", file.name))?;

            // Decompress LZSS if needed
            let final_data = if file.compressed {
                lzss::decompress(&file_data)
                    .with_context(|| format!("Failed to decompress {}", file.name))?
            } else {
                file_data
            };

            fs::write(&output_path, final_data)
                .with_context(|| format!("Failed to write {}", output_path.display()))?;
        }

        Ok(())
    }

    /// Read file data from the raw archive bytes
    fn read_file_data(&self, file: &FileEntry) -> Result<Vec<u8>> {
        if let Some(ref data) = file.data {
            return Ok(data.clone());
        }

        let start = file.offset as usize;
        let end = start + file.packed_size as usize;

        if end > self.data.len() {
            bail!("File data extends beyond archive: {}", file.name);
        }

        Ok(self.data[start..end].to_vec())
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
        let mut output = Vec::new();
        let mut cursor = Cursor::new(&mut output);

        // Write 16-byte header
        cursor.write_u32::<BigEndian>(self.directories.len() as u32)?;
        cursor.write_u32::<BigEndian>(DAT1_FORMAT_ID)?; // unknown1: format identifier seen in original files
        cursor.write_u32::<BigEndian>(0)?; // unknown2: always zero in practice
        cursor.write_u32::<BigEndian>(0)?; // unknown3: always zero in practice

        // Write directory names
        for dir in &self.directories {
            cursor.write_u8(dir.name.len() as u8)?;
            cursor.write_all(dir.name.as_bytes())?;
        }

        // Calculate where file data starts (after all directory content headers)
        let mut data_offset = cursor.position() as u32;
        for dir in &self.directories {
            data_offset += 16; // Directory header: file_count + 3 unknown fields
            for file in &dir.files {
                let file_name_len =
                    if file.name.starts_with(&format!("{}\\", dir.name)) && dir.name != "." {
                        file.name.len() - dir.name.len() - 1
                    } else {
                        file.name.len()
                    };
                data_offset += 1 + file_name_len as u32 + 16; // name_len byte + name + entry fields
            }
        }

        let mut current_offset = data_offset;

        // Write directory content headers and file entries
        for dir in &self.directories {
            cursor.write_u32::<BigEndian>(dir.files.len() as u32)?;
            cursor.write_u32::<BigEndian>(DAT1_FORMAT_ID)?;
            cursor.write_u32::<BigEndian>(DAT1_DIRECTORY_UNKNOWN5)?;
            cursor.write_u32::<BigEndian>(0)?;

            for file in &dir.files {
                // Strip directory prefix from filename for storage
                let file_name =
                    if file.name.starts_with(&format!("{}\\", dir.name)) && dir.name != "." {
                        &file.name[dir.name.len() + 1..]
                    } else {
                        &file.name
                    };

                cursor.write_u8(file_name.len() as u8)?;
                cursor.write_all(file_name.as_bytes())?;

                let attributes = if file.compressed {
                    DAT1_COMPRESSED_FLAG
                } else {
                    DAT1_UNCOMPRESSED_FLAG
                };
                cursor.write_u32::<BigEndian>(attributes)?;
                cursor.write_u32::<BigEndian>(current_offset)?;
                cursor.write_u32::<BigEndian>(file.size)?;
                cursor.write_u32::<BigEndian>(if file.compressed {
                    file.packed_size
                } else {
                    0
                })?;

                current_offset += file.packed_size;
            }
        }

        // Write file data
        for dir in &self.directories {
            for file in &dir.files {
                let data = if let Some(ref file_data) = file.data {
                    file_data.clone() // File data is already in memory (newly added file)
                } else {
                    self.read_file_data(file)? // Need to read from the original archive
                };
                cursor.write_all(&data)?;
            }
        }

        fs::write(path, output).context("Failed to write DAT1 file")?;

        Ok(())
    }
}
