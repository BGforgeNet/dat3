/*!
# DAT2 Archive Format Implementation

This module implements support for the Fallout 2 DAT2 archive format.

## Format Overview

DAT2 is the archive format used by Fallout 2, with these characteristics:

- **Endianness**: Little-endian (unlike DAT1's big-endian)
- **Structure**: Flat file list (no hierarchical directories)
- **Compression**: zlib compression for individual files
- **Performance**: Optimized for parallel extraction

## File Structure

```
DAT2 Archive Layout:
1. File data (variable length)
   - All files concatenated in order
   - Each file may be compressed with zlib
2. Directory tree (variable length)
   - File count (4 bytes, little-endian)
   - File entries (variable length each)
3. Footer (8 bytes)
   - Tree size (4 bytes) - size of directory tree + file count
   - DAT size (4 bytes) - total archive size
```

## Performance Features

- **Parallel extraction**: Uses rayon for multi-threaded file processing
- **Memory efficiency**: Pre-allocated buffers based on known file sizes
- **Progress reporting**: Real-time extraction progress for large archives

## Implementation Notes

- Files are sorted alphabetically in the archive (as per DAT2 format)
- Compression is optional and only used when it saves space
- Standard DAT2 format compatibility
*/

#![allow(clippy::manual_div_ceil)]

// Standard library and external crates
use anyhow::{bail, Context, Result}; // Error handling
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt}; // Reading binary data in little-endian format
use deku::prelude::*; // Declarative binary parsing
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression}; // zlib compression
use rayon::prelude::*; // Parallel processing
use std::collections::HashSet; // For deduplication
use std::fs; // File system operations
use std::io::{Cursor, Read, Write}; // Input/output operations
use std::path::Path; // Cross-platform paths
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
}; // Thread-safe shared data
use std::time::Instant; // Performance timing

use crate::common::{utils, CompressionLevel, FileEntry};

/// Footer that appears at the end of every DAT2 file (8 bytes total)
/// This tells us where to find the file directory and validates the file size
#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "little")] // DAT2 uses little-endian byte order
struct Dat2Footer {
    tree_size: u32, // Size of the directory tree data
    dat_size: u32,  // Total size of the entire DAT file
}

/// Single file entry as stored in the DAT2 directory tree
/// Each file in the archive has one of these records
#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "little")] // DAT2 uses little-endian byte order
struct Dat2FileEntry {
    filename_size: u32, // Length of the filename in bytes
    #[deku(count = "filename_size")] // Read exactly filename_size bytes
    filename_bytes: Vec<u8>, // The filename as raw bytes (may not be UTF-8)
    compression_type: u8, // 0 = uncompressed, 1 = zlib compressed
    real_size: u32,     // Original file size (before compression)
    packed_size: u32,   // Compressed size (or same as real_size if not compressed)
    offset: u32,        // Position in the DAT file where this file's data starts
}

/// Main DAT2 archive handler
///
/// This struct manages the entire Fallout 2 DAT2 archive, providing high-level
/// operations for reading, writing, and manipulating DAT2 files.
///
/// ## Key Features
/// - **Fast extraction**: Parallel processing with rayon
/// - **Memory efficient**: Optimized for large archives
/// - **Standard format**: Works with DAT2 files
///
/// ## Usage Example
/// ```ignore
/// let archive = Dat2Archive::from_bytes(file_data)?;
/// archive.extract("./output", &[], false)?; // Extract all files
/// ```
#[derive(Debug)]
pub struct Dat2Archive {
    /// All file entries in the archive (metadata and data references)
    files: Vec<FileEntry>,
    /// Raw archive data (the entire DAT file contents)
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

    /// Load DAT2 archive from bytes
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        if data.len() < 8 {
            bail!("DAT2 file too small");
        }

        let files = Self::parse_directory_tree(&data)?;

        Ok(Self { files, data })
    }

    /// Parse directory tree from data slice
    fn parse_directory_tree(data: &[u8]) -> Result<Vec<FileEntry>> {
        // Parse footer using deku
        let footer_bytes = &data[data.len() - 8..];
        let (_, footer) = Dat2Footer::from_bytes((footer_bytes, 0))
            .map_err(|e| anyhow::anyhow!("Failed to parse DAT2 footer: {}", e))?;

        // Validate file size matches
        if footer.dat_size as usize != data.len() {
            bail!(
                "DAT size mismatch: expected {}, got {}",
                footer.dat_size,
                data.len()
            );
        }

        // Calculate directory tree position (includes FilesTotal + DirTree)
        let tree_start = footer.dat_size as usize - footer.tree_size as usize - 8;
        if tree_start < 4 {
            bail!("Invalid directory tree position");
        }

        // Read file count first (using byteorder for simplicity)
        let mut cursor = Cursor::new(&data[tree_start..]);
        let file_count = cursor.read_u32::<LittleEndian>()?;

        // Parse directory tree entries using deku
        let mut files = Vec::with_capacity(file_count as usize);
        let tree_data = &data[tree_start + 4..data.len() - 8]; // Skip file count
        let mut current_offset = 0;

        for _ in 0..file_count {
            let remaining_data = &tree_data[current_offset..];
            let ((remaining_slice, _bit_offset), entry) =
                Dat2FileEntry::from_bytes((remaining_data, 0))
                    .map_err(|e| anyhow::anyhow!("Failed to parse file entry: {}", e))?;

            let filename = utils::decode_filename(&entry.filename_bytes)
                .context("Failed to decode filename")?;

            files.push(FileEntry {
                name: filename.replace('\\', "/"), // Convert archive format to internal format
                offset: entry.offset as u64,
                size: entry.real_size,
                packed_size: entry.packed_size,
                compressed: entry.compression_type == 1,
                data: None,
            });

            // Calculate how many bytes were consumed
            let bytes_consumed = remaining_data.len() - remaining_slice.len();
            current_offset += bytes_consumed;
        }

        Ok(files)
    }

    /// Get data slice for reading files
    fn get_data_slice(&self) -> &[u8] {
        &self.data
    }

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, files: &[String]) -> Result<()> {
        // Normalize user input patterns to internal format (forward slashes)
        let normalized_patterns: Vec<String> = files
            .iter()
            .map(|p| utils::normalize_user_path(p).into_owned())
            .collect();

        // Use shared filtering logic
        let (files_to_list, missing_patterns) = crate::common::filter_and_track_patterns(
            &self.files,
            &normalized_patterns,
            |file, pattern| file.name.contains(pattern),
        );

        println!("{:>11} {:>11}  {:>4}  Name", "Size", "Packed", "Comp");
        println!("{}", "-".repeat(50));

        for file in files_to_list {
            let comp_str = if file.compressed { "Yes" } else { "No" };
            let display_name = utils::normalize_path_for_display(&file.name);
            println!(
                "{:>11} {:>11}  {:>4}  {}",
                file.size, file.packed_size, comp_str, display_name
            );
        }

        // Report missing patterns
        if !missing_patterns.is_empty() {
            eprintln!("\nFiles not found:");
            for pattern in &missing_patterns {
                eprintln!("  {pattern}");
            }
            bail!("Some requested files were not found");
        }

        Ok(())
    }

    /// Extract files from the archive to a directory
    ///
    /// Arguments:
    /// - output_dir: Where to extract files
    /// - files: Specific files to extract (empty = extract all)
    /// - flat: If true, ignore directory structure and put all files in output_dir
    pub fn extract<P: AsRef<Path>>(
        &self,
        output_dir: P,
        files: &[String],
        flat: bool,
    ) -> Result<()> {
        let output_dir = output_dir.as_ref();

        // Normalize user input patterns to internal format (forward slashes)
        let normalized_patterns: Vec<String> = files
            .iter()
            .map(|p| utils::normalize_user_path(p).into_owned())
            .collect();

        // Decide which files to extract
        let files_to_extract: Vec<&FileEntry> = self
            .files
            .iter()
            .filter(|file| {
                // If no specific files requested, extract everything
                // Otherwise, extract files whose names contain any of the requested patterns
                normalized_patterns.is_empty()
                    || normalized_patterns
                        .iter()
                        .any(|pattern| file.name.contains(pattern))
            })
            .collect();

        // Share archive data across parallel threads for better performance
        let archive_data = Arc::new(self.get_data_slice());
        let total_files = files_to_extract.len();
        let completed = Arc::new(AtomicUsize::new(0));

        println!("Extracting {total_files} files...");
        let start = Instant::now();

        // Extract files in parallel for better performance
        files_to_extract
            .par_iter()
            .try_for_each(|file| -> Result<()> {
                // Show progress every 1000 files or at the end
                let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 1000 == 0 || count == total_files {
                    let elapsed = start.elapsed().as_millis();
                    let files_per_sec = count as f64 / elapsed as f64 * 1000.0;
                    println!(
                        "Progress: {count}/{total_files} files extracted ({files_per_sec:.1} files/sec)"
                    );
                }

                // Determine where to save this file
                let output_path = if flat {
                    // Flat extraction: extract just the filename without directory path
                    let filename = utils::get_filename_from_dat_path(&file.name);
                    output_dir.join(filename)
                } else {
                    // Preserve directory structure from the archive
                    output_dir.join(utils::to_system_path(&file.name))
                };

                // Create directories if they don't exist
                utils::ensure_dir_exists(&output_path)?;

                // Read the file data from the archive
                let file_data = self.read_file_data_from_bytes(&archive_data, file)?;

                // Decompress if needed, otherwise use data as-is
                let final_data = if file.compressed {
                    Self::decompress_zlib_static_with_size(&file_data, file.size as usize)
                        .with_context(|| format!("Failed to decompress {}", file.name))?
                } else {
                    file_data
                };

                // Write the file to disk
                fs::write(&output_path, final_data)
                    .with_context(|| format!("Failed to write {}", output_path.display()))?;

                Ok(())
            })?;

        // Show completion message
        let total_time = start.elapsed();
        println!("Extraction completed in {:.2}s", total_time.as_secs_f64());

        Ok(())
    }

    /// Read file data from shared archive bytes (thread-safe)
    fn read_file_data_from_bytes(&self, archive_data: &[u8], file: &FileEntry) -> Result<Vec<u8>> {
        if let Some(ref data) = file.data {
            return Ok(data.clone());
        }

        let start = file.offset as usize;
        let end = start + file.packed_size as usize;

        if end > archive_data.len() {
            bail!(
                "File data extends beyond archive: {} (offset: {}, size: {})",
                file.name,
                file.offset,
                file.packed_size
            );
        }

        Ok(archive_data[start..end].to_vec())
    }

    /// Optimized zlib decompression with pre-allocated buffer
    fn decompress_zlib_static_with_size(data: &[u8], expected_size: usize) -> Result<Vec<u8>> {
        let mut decoder = ZlibDecoder::new(data);
        let mut decompressed = Vec::with_capacity(expected_size);
        decoder
            .read_to_end(&mut decompressed)
            .context("Failed to decompress zlib data")?;
        Ok(decompressed)
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

    /// Compress data with zlib
    fn compress_zlib_static(data: &[u8], level: u8) -> Result<Vec<u8>> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level as u32));
        encoder.write_all(data)?;
        encoder.finish().context("Failed to compress with zlib")
    }

    /// Add a file to the archive
    pub fn add_file<P: AsRef<Path>>(
        &mut self,
        file_path: P,
        recursive: bool,
        compression: CompressionLevel,
        target_dir: Option<&str>,
    ) -> Result<()> {
        let base_path = file_path.as_ref();
        let files = utils::collect_files(&file_path, recursive)?;

        // Helper closure to determine archive path
        let get_archive_path = |file: &std::path::PathBuf| -> Result<String> {
            let archive_path = if let Some(target) = target_dir {
                if recursive && base_path.is_dir() {
                    // Preserve directory structure including the base directory name
                    let relative_path = if let Some(parent) = base_path.parent() {
                        file.strip_prefix(parent).unwrap_or(file).to_string_lossy()
                    } else {
                        file.to_string_lossy()
                    };
                    format!("{target}/{relative_path}")
                } else {
                    format!(
                        "{target}/{}",
                        file.file_name()
                            .ok_or_else(|| anyhow::anyhow!(
                                "Invalid filename for: {}",
                                file.display()
                            ))?
                            .to_string_lossy()
                    )
                }
            } else if recursive && base_path.is_dir() {
                // Preserve directory structure including the base directory name
                if let Some(parent) = base_path.parent() {
                    file.strip_prefix(parent)
                        .unwrap_or(file)
                        .to_string_lossy()
                        .to_string()
                } else {
                    file.to_string_lossy().to_string()
                }
            } else {
                file.file_name()
                    .ok_or_else(|| anyhow::anyhow!("Invalid filename for: {}", file.display()))?
                    .to_string_lossy()
                    .to_string()
            };
            Ok(utils::normalize_path_for_archive(&archive_path))
        };

        // Process files in parallel
        let results: Result<Vec<FileEntry>> = files
            .par_iter()
            .map(|file| {
                let data =
                    fs::read(file).with_context(|| format!("Failed to read {}", file.display()))?;

                let archive_path = get_archive_path(file)?;
                let display_path = utils::normalize_path_for_display(&archive_path);
                println!("Adding: {display_path}");

                // Compress if requested and create file entry
                let file_entry = if compression.level() > 0 {
                    let compressed_data = Self::compress_zlib_static(&data, compression.level())?;

                    // Only use compression if it actually saves space
                    if compressed_data.len() < data.len() {
                        FileEntry::with_compression_data(archive_path, data, compressed_data)
                    } else {
                        let mut entry = FileEntry::with_data(archive_path, data, false);
                        entry.size = entry.packed_size; // Set size for uncompressed
                        entry
                    }
                } else {
                    let mut entry = FileEntry::with_data(archive_path, data, false);
                    entry.size = entry.packed_size; // Set size for uncompressed
                    entry
                };

                Ok(file_entry)
            })
            .collect();

        let new_entries = results?;

        // Remove existing files with same names and add new entries
        let mut seen_names = HashSet::new();
        for entry in new_entries {
            if seen_names.insert(entry.name.clone()) {
                // Remove existing file with same name from archive
                self.files.retain(|f| f.name != entry.name);
                self.files.push(entry);
            }
        }

        // Sort files alphabetically (as per DAT2 format)
        self.files.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(())
    }

    /// Delete a file from the archive
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        // Normalize user input to internal format (forward slashes)
        let normalized_name = utils::normalize_user_path(file_name).into_owned();

        if let Some(pos) = self.files.iter().position(|f| f.name == normalized_name) {
            let display_name = utils::normalize_path_for_display(&normalized_name);
            println!("Deleting: {display_name}");
            self.files.remove(pos);
            Ok(())
        } else {
            bail!("File not found: {}", file_name);
        }
    }

    /// Save the archive to a file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let mut output = Vec::new();
        let mut cursor = Cursor::new(&mut output);

        // Write file data first
        let mut current_offset = 0u32;
        let mut file_offsets = Vec::new();

        for file in &self.files {
            file_offsets.push(current_offset);

            let data = if let Some(ref file_data) = file.data {
                file_data.clone()
            } else {
                self.read_file_data(file)?
            };

            cursor.write_all(&data)?;
            current_offset += data.len() as u32;
        }

        // Mark start of directory tree area (includes file count + entries)
        let tree_start = cursor.position();

        // Write file count
        cursor.write_u32::<LittleEndian>(self.files.len() as u32)?;

        // Write directory tree using deku
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
        }

        let tree_end = cursor.position();
        let tree_size = (tree_end - tree_start) as u32;
        let total_size = tree_end + 8;

        // Write footer using deku
        let footer = Dat2Footer {
            tree_size,
            dat_size: total_size as u32,
        };
        let footer_bytes = footer.to_bytes()?;
        cursor.write_all(&footer_bytes)?;

        fs::write(path, output).context("Failed to write DAT2 file")?;

        Ok(())
    }
}
