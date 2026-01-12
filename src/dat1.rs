/*!
# DAT1 Archive Format Implementation

This module implements support for the Fallout 1 DAT1 archive format.

## Format Overview

DAT1 is the archive format used by Fallout 1, with these characteristics:

- **Endianness**: Big-endian (unlike DAT2's little-endian)
- **Structure**: Hierarchical directory structure
- **Compression**: LZSS compression (currently stored uncompressed in our implementation)
- **Compatibility**: Standard DAT1 format

## File Structure

```
DAT1 Archive Layout:
1. Header (16 bytes)
   - Directory count (4 bytes, big-endian)
   - 3 unknown fields (12 bytes)
2. Directory names (variable length)
   - For each directory: length byte + name
3. Directory contents (variable length)
   - For each directory: header + file entries
4. File data (variable length)
   - Raw file content, stored in order
```

## Implementation Notes

- Files are stored uncompressed since LZSS compression is not implemented
- Directory paths use backslashes as per DAT1 format
- File offsets must be calculated correctly for extraction to work
*/

use anyhow::{bail, Context, Result};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use crate::common::{utils, ArchiveFormat, CompressionLevel, ExtractionMode, FileEntry};
use crate::lzss;

// DAT1 format constants
const DAT1_COMPRESSED_FLAG: u32 = 0x40;
const DAT1_UNCOMPRESSED_FLAG: u32 = 0x20;
const DAT1_FORMAT_ID: u32 = 0x0A;
const DAT1_DIRECTORY_UNKNOWN5: u32 = 0x10;

/// Represents a directory within a DAT1 archive
///
/// DAT1 uses a hierarchical directory structure where each directory
/// contains a list of files. The root directory is named "."
#[derive(Debug, Clone)]
struct Directory {
    /// Directory name (e.g., "CRITTERS", "SOUND", or "." for root)
    name: String,
    /// Files contained in this directory
    files: Vec<FileEntry>,
}

/// Main DAT1 archive handler
///
/// This struct manages the entire Fallout 1 DAT1 archive, including:
/// - Directory structure (hierarchical)
/// - File metadata and data
/// - Reading from and writing to DAT1 files
#[derive(Debug)]
pub struct Dat1Archive {
    /// All directories in the archive (including root directory ".")
    directories: Vec<Directory>,
    /// Raw file data for the entire archive (used when reading existing archives)
    data: Vec<u8>,
}

impl Dat1Archive {
    /// Create a new empty DAT1 archive
    ///
    /// This creates a fresh archive with just a root directory (".").
    /// Files can then be added using the `add_file` method.
    pub fn new() -> Self {
        Self {
            directories: vec![Directory {
                name: ".".to_string(), // Root directory in DAT1 format
                files: Vec::new(),
            }],
            data: Vec::new(),
        }
    }

    /// Load an existing DAT1 archive from raw bytes
    ///
    /// This parses the DAT1 format and creates an archive object that can be
    /// used to list, extract, or modify files. The parsing handles:
    /// - Reading the header and directory count
    /// - Parsing directory names and file entries
    /// - Setting up file metadata for extraction
    ///
    /// # Arguments
    /// * `data` - The complete DAT1 file as a byte vector
    ///
    /// # Returns
    /// * `Result<Self>` - The parsed archive or an error if the format is invalid
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        let mut cursor = Cursor::new(&data);

        // Read header
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

        let mut directories = Vec::new();

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

        // Read directory contents
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

                // Build full path
                let full_name = if dir_name == "." {
                    name
                } else {
                    format!("{dir_name}\\{name}")
                };

                files.push(FileEntry {
                    name: full_name, // Already using backslashes consistently
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

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, files: &[String]) -> Result<()> {
        // Normalize user input patterns to internal format (backslashes)
        let normalized_patterns = utils::normalize_user_patterns(files);

        // Flatten all files from all directories for filtering
        let all_files: Vec<&FileEntry> =
            self.directories.iter().flat_map(|dir| &dir.files).collect();

        // Use shared filtering logic
        let (files_to_list, missing_patterns) = crate::common::filter_and_track_patterns(
            &all_files,
            &normalized_patterns,
            |file, pattern| file.name.contains(pattern),
        );

        utils::print_file_listing(&files_to_list);

        // Report missing patterns
        if !missing_patterns.is_empty() {
            eprintln!("\nFiles not found:");
            for pattern in &missing_patterns {
                let display = utils::normalize_path_for_display(pattern);
                eprintln!("  {display}");
            }
            bail!("Some requested files were not found");
        }

        Ok(())
    }

    /// Extract files from the archive
    pub fn extract<P: AsRef<Path>>(
        &self,
        output_dir: P,
        files: &[String],
        mode: ExtractionMode,
    ) -> Result<()> {
        let output_dir = output_dir.as_ref();

        // Normalize user input patterns to internal format (backslashes)
        let normalized_patterns = utils::normalize_user_patterns(files);

        for dir in &self.directories {
            for file in &dir.files {
                // Check if we should extract this file
                if !normalized_patterns.is_empty()
                    && !normalized_patterns.iter().any(|f| file.name.contains(f))
                {
                    continue;
                }

                let display_name = utils::normalize_path_for_display(&file.name);
                println!("Extracting: {display_name}");

                let output_path = match mode {
                    ExtractionMode::Flat => {
                        // Flat extraction: extract just the filename without directory path
                        let filename = utils::get_filename_from_dat_path(&file.name);
                        output_dir.join(filename)
                    }
                    ExtractionMode::PreserveStructure => {
                        output_dir.join(utils::to_system_path(&file.name))
                    }
                };

                utils::ensure_dir_exists(&output_path)?;

                // Read file data from archive
                let file_data = self
                    .read_file_data(file)
                    .with_context(|| format!("Failed to read data for file '{}'", file.name))?;

                // Decompress if needed
                let final_data = if file.compressed {
                    lzss::decompress(&file_data)
                        .with_context(|| format!("Failed to decompress {}", file.name))?
                } else {
                    file_data
                };

                fs::write(&output_path, final_data)
                    .with_context(|| format!("Failed to write {}", output_path.display()))?;
            }
        }

        Ok(())
    }

    /// Read file data from the archive
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

    /// Add files to the archive
    ///
    /// This function adds one or more files to the DAT1 archive. It handles both
    /// individual files and directories (always processed recursively).
    ///
    /// **Important**: DAT1 compression is not implemented, so files are stored
    /// uncompressed regardless of the compression parameter.
    ///
    /// # Arguments
    /// * `file_path` - Path to the file or directory to add
    /// * `_compression` - Compression level (ignored for DAT1)
    /// * `target_dir` - Optional directory path within the archive
    ///
    /// # Example
    /// ```ignore
    /// let compression = CompressionLevel::new(6)?;
    /// archive.add_file("image.png", compression, Some("GRAPHICS"))?;
    /// // Adds image.png to GRAPHICS/image.png in the archive
    /// ```
    pub fn add_file<P: AsRef<Path>>(
        &mut self,
        file_path: P,
        _compression: CompressionLevel,
        target_dir: Option<&str>,
        strip_leading_directory: bool,
    ) -> Result<()> {
        let base_path = file_path.as_ref();
        let files = utils::collect_files(&file_path).with_context(|| {
            format!(
                "Failed to collect files from path '{}'",
                file_path.as_ref().display()
            )
        })?;

        for file in files {
            let data =
                fs::read(&file).with_context(|| format!("Failed to read {}", file.display()))?;

            // Calculate archive path using common logic with stripping support
            let archive_path = utils::calculate_archive_path(
                &file,
                base_path,
                target_dir,
                strip_leading_directory,
            )?;

            // DAT1 stores files uncompressed
            let size = data.len() as u32;
            let (final_data, compressed, original_size) = (data, false, size);

            let display_path = utils::normalize_path_for_display(&archive_path);
            println!("Adding: {display_path}");

            // Find or create target directory
            let dir_name = utils::get_dirname_from_dat_path(&archive_path);

            // Find directory or create new one
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

            // Add file entry
            let mut file_entry = FileEntry::with_data(archive_path, final_data, compressed);
            if !compressed {
                // For uncompressed files, size equals packed_size
                file_entry.size = file_entry.packed_size;
            } else {
                // For compressed files, set the original size
                file_entry.size = original_size;
            }
            self.directories[dir_index].files.push(file_entry);
        }

        Ok(())
    }

    /// Delete a file from the archive
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        // Normalize user input to internal format (backslashes)
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
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let mut output = Vec::new();
        let mut cursor = Cursor::new(&mut output);

        // Write header
        cursor.write_u32::<BigEndian>(self.directories.len() as u32)?;
        cursor.write_u32::<BigEndian>(DAT1_FORMAT_ID)?; // unknown1
        cursor.write_u32::<BigEndian>(0)?; // unknown2
        cursor.write_u32::<BigEndian>(0)?; // unknown3

        // Write directory names
        for dir in &self.directories {
            cursor.write_u8(dir.name.len() as u8)?;
            cursor.write_all(dir.name.as_bytes())?;
        }

        // Calculate data start position
        let mut data_offset = cursor.position() as u32;

        // Add space for directory contents headers
        for dir in &self.directories {
            data_offset += 16; // Directory header
            for file in &dir.files {
                let file_name_len =
                    if file.name.starts_with(&format!("{}\\", dir.name)) && dir.name != "." {
                        file.name.len() - dir.name.len() - 1 // Subtract directory name and backslash
                    } else {
                        file.name.len()
                    };
                data_offset += 1 + file_name_len as u32 + 16; // File entry
            }
        }

        let mut current_offset = data_offset;

        // Write directory contents
        for dir in &self.directories {
            cursor.write_u32::<BigEndian>(dir.files.len() as u32)?;
            cursor.write_u32::<BigEndian>(DAT1_FORMAT_ID)?; // unknown4
            cursor.write_u32::<BigEndian>(DAT1_DIRECTORY_UNKNOWN5)?; // unknown5
            cursor.write_u32::<BigEndian>(0)?; // unknown6

            for file in &dir.files {
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
                    file_data.clone()
                } else {
                    self.read_file_data(file)?
                };
                cursor.write_all(&data)?;
            }
        }

        fs::write(path, output).context("Failed to write DAT1 file")?;

        Ok(())
    }
}

/// ArchiveFormat trait implementation for DAT1 (Fallout 1) archives.
///
/// Delegates to the inherent methods on Dat1Archive. Note that compression
/// is ignored for DAT1 - files are always stored uncompressed since LZSS
/// compression is not implemented.
impl ArchiveFormat for Dat1Archive {
    fn list(&self, files: &[String]) -> Result<()> {
        Dat1Archive::list(self, files)
    }

    fn extract(&self, output_dir: &Path, files: &[String], mode: ExtractionMode) -> Result<()> {
        Dat1Archive::extract(self, output_dir, files, mode)
    }

    fn add_file(
        &mut self,
        file_path: &Path,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        strip_leading_directory: bool,
    ) -> Result<()> {
        Dat1Archive::add_file(
            self,
            file_path,
            compression,
            target_dir,
            strip_leading_directory,
        )
    }

    fn delete_file(&mut self, file_name: &str) -> Result<()> {
        Dat1Archive::delete_file(self, file_name)
    }

    fn save(&self, path: &Path) -> Result<()> {
        Dat1Archive::save(self, path)
    }
}
