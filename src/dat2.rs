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

use crate::common::{utils, CompressionLevel, ExtractionMode, FileEntry};

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
        let file_count = cursor
            .read_u32::<LittleEndian>()
            .context("Failed to read file count from DAT2 directory tree")?;

        // Parse directory tree entries using deku
        let mut files = Vec::with_capacity(file_count as usize);
        let tree_data = &data[tree_start + 4..data.len() - 8]; // Skip file count
        let mut current_offset = 0;

        for i in 0..file_count {
            let remaining_data = &tree_data[current_offset..];
            let ((remaining_slice, _bit_offset), entry) =
                Dat2FileEntry::from_bytes((remaining_data, 0))
                    .map_err(|e| anyhow::anyhow!("Failed to parse file entry: {}", e))?;

            let filename = utils::decode_filename(&entry.filename_bytes)
                .with_context(|| format!("Failed to decode filename for file entry {i}"))?;

            files.push(FileEntry {
                name: filename, // Keep backslashes for internal consistency
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
        // Normalize user input patterns to internal format (backslashes)
        let normalized_patterns = utils::normalize_user_patterns(files);

        // Use shared filtering logic
        let (files_to_list, missing_patterns) = crate::common::filter_and_track_patterns(
            &self.files,
            &normalized_patterns,
            |file, pattern| file.name.contains(pattern),
        );

        utils::print_file_listing(&files_to_list);

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
    /// - mode: Extraction mode (preserve structure or flat)
    pub fn extract<P: AsRef<Path>>(
        &self,
        output_dir: P,
        files: &[String],
        mode: ExtractionMode,
    ) -> Result<()> {
        let output_dir = output_dir.as_ref();

        // Use shared filtering logic from common module
        let normalized_patterns = utils::normalize_user_patterns(files);

        let (files_to_extract, _) = crate::common::filter_and_track_patterns(
            &self.files,
            &normalized_patterns,
            |file, pattern| file.name.contains(pattern),
        );

        self.extract_files_parallel(&files_to_extract, output_dir, mode)?;

        Ok(())
    }

    /// Extract files in parallel (helper method)
    fn extract_files_parallel(
        &self,
        files_to_extract: &[&FileEntry],
        output_dir: &Path,
        mode: ExtractionMode,
    ) -> Result<()> {
        let archive_data = Arc::new(self.get_data_slice());
        let total_files = files_to_extract.len();
        let completed = Arc::new(AtomicUsize::new(0));

        println!("Extracting {total_files} files...");
        let start = Instant::now();

        files_to_extract
            .par_iter()
            .try_for_each(|file| -> Result<()> {
                // Show progress
                let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                if count % 1000 == 0 || count == total_files {
                    let elapsed = start.elapsed().as_millis();
                    let files_per_sec = count as f64 / elapsed as f64 * 1000.0;
                    println!(
                        "Progress: {count}/{total_files} files extracted ({files_per_sec:.1} files/sec)"
                    );
                }

                // Determine output path
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

                // Read and decompress file data
                let file_data = self.read_file_data_from_bytes(&archive_data, file)?;
                let final_data = if file.compressed {
                    Self::decompress_zlib_static_with_size(&file_data, file.size as usize)
                        .with_context(|| format!("Failed to decompress {}", file.name))?
                } else {
                    file_data
                };

                fs::write(&output_path, final_data)
                    .with_context(|| format!("Failed to write {}", output_path.display()))?;

                Ok(())
            })?;

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

    /// Compress data using zlib compression
    ///
    /// This is a helper function that takes raw file data and compresses it.
    /// The compression level determines how much CPU time to spend on compression:
    /// - Level 0: No compression (fastest)
    /// - Level 9: Maximum compression (slowest)
    fn compress_zlib_static(data: &[u8], level: u8) -> Result<Vec<u8>> {
        // Create a new zlib encoder that writes to a Vec<u8>
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level as u32));

        // Write all the data to the encoder
        encoder.write_all(data)?;

        // Finish compression and get the compressed data
        encoder.finish().context("Failed to compress with zlib")
    }

    /// Process a single file for adding to archive (helper method)
    fn process_single_file_for_adding(
        &self,
        file: &std::path::Path,
        base_path: &std::path::Path,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        strip_leading_directory: bool,
    ) -> Result<FileEntry> {
        let data = fs::read(file).with_context(|| format!("Failed to read {}", file.display()))?;
        let archive_path =
            utils::calculate_archive_path(file, base_path, target_dir, strip_leading_directory)?;
        let display_path = utils::normalize_path_for_display(&archive_path);
        println!("Adding: {display_path}");

        if compression.level() > 0 {
            let compressed_data = Self::compress_zlib_static(&data, compression.level())?;
            if compressed_data.len() < data.len() {
                Ok(FileEntry::with_compression_data(
                    archive_path,
                    data,
                    compressed_data,
                ))
            } else {
                let mut entry = FileEntry::with_data(archive_path, data, false);
                entry.size = entry.packed_size;
                Ok(entry)
            }
        } else {
            let mut entry = FileEntry::with_data(archive_path, data, false);
            entry.size = entry.packed_size;
            Ok(entry)
        }
    }

    /// Add files to the archive (directories processed recursively)
    ///
    /// This method can add a single file or an entire directory to the archive.
    /// Files are processed in parallel for better performance.
    ///
    /// # Parameters
    /// - `file_path`: Path to file or directory to add
    /// - `compression`: How much to compress files (0=none, 9=maximum)
    /// - `target_dir`: Optional directory name inside the archive to put files
    ///
    /// # Examples
    /// ```ignore
    /// // Add a single file
    /// archive.add_file("myfile.txt", CompressionLevel::new(6)?, None)?;
    ///
    /// // Add a directory with compression
    /// archive.add_file("my_folder", CompressionLevel::new(1)?, Some("data"))?;
    /// ```
    pub fn add_file<P: AsRef<Path>>(
        &mut self,
        file_path: P,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        strip_leading_directory: bool,
    ) -> Result<()> {
        let base_path = file_path.as_ref();

        // Find all files to add (handles both single files and directories)
        let files = utils::collect_files(&file_path).with_context(|| {
            format!(
                "Failed to collect files from path '{}'",
                file_path.as_ref().display()
            )
        })?;

        // Process all files in parallel for better performance
        let results: Result<Vec<FileEntry>> = files
            .par_iter()
            .map(|file| {
                self.process_single_file_for_adding(
                    file,
                    base_path,
                    compression,
                    target_dir,
                    strip_leading_directory,
                )
            })
            .collect();

        // Get the processed file entries (this will error if any file failed)
        let new_entries = results?;

        // Add all the new files to the archive, handling duplicates
        // First, remove any existing files from archive that match the new file names
        let new_file_names: HashSet<String> = new_entries.iter().map(|e| e.name.clone()).collect();
        self.files
            .retain(|existing_file| !new_file_names.contains(&existing_file.name));

        // Then add new files, deduplicating within the new batch
        let mut seen_names = HashSet::new();
        for entry in new_entries {
            // Only add if we haven't seen this filename already in this batch
            if seen_names.insert(entry.name.clone()) {
                self.files.push(entry);
            }
            // If we've seen this name before in this batch, skip it (keeps first occurrence)
        }

        // DAT2 format requires files to be sorted alphabetically (case-insensitive)
        self.files
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        Ok(())
    }

    /// Delete a file from the archive
    ///
    /// Removes a file from the archive by name. The file name should match
    /// what you see when listing the archive contents.
    ///
    /// # Parameters
    /// - `file_name`: Name of the file to delete (can use forward or back slashes)
    ///
    /// # Examples
    /// ```ignore
    /// archive.delete_file("data/myfile.txt")?;
    /// archive.delete_file("data\\myfile.txt")?;  // Also works
    /// ```
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        // Convert the user's input to the internal format used by the archive
        // (handles both forward and back slashes, converts to backslashes internally)
        let normalized_name = utils::normalize_user_path(file_name).into_owned();

        // Look for a file with this name in the archive
        if let Some(position) = self
            .files
            .iter()
            .position(|file| file.name == normalized_name)
        {
            let display_name = utils::normalize_path_for_display(&normalized_name);
            println!("Deleting: {display_name}");

            // Remove the file from the list
            self.files.remove(position);
            Ok(())
        } else {
            // File not found - this is an error
            bail!("File not found: {}", file_name);
        }
    }

    /// Save the archive to disk
    ///
    /// This writes the entire archive to a DAT2 file. The file will be written
    /// in the correct DAT2 format that game engines can read.
    ///
    /// # DAT2 File Structure
    /// 1. All file data (compressed or uncompressed)
    /// 2. Directory tree (file count + list of file entries)
    /// 3. Footer (tree size + total file size)
    ///
    /// # Parameters
    /// - `path`: Where to save the DAT2 file
    ///
    /// # Examples
    /// ```ignore
    /// archive.save("my_archive.dat")?;
    /// archive.save("C:/games/fallout2/master.dat")?;
    /// ```
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        // We build the entire file in memory, then write it all at once
        let mut output = Vec::new();
        let mut cursor = Cursor::new(&mut output);

        // === STEP 1: Write all file data ===
        // This goes at the beginning of the DAT2 file
        let mut current_offset = 0u32;
        let mut file_offsets = Vec::new(); // Track where each file starts

        for file in &self.files {
            // Remember where this file starts in the archive
            file_offsets.push(current_offset);

            // Get the file's data (either from memory or read from original archive)
            let data = if let Some(ref file_data) = file.data {
                // File data is already in memory (newly added file)
                file_data.clone()
            } else {
                // Need to read from the original archive
                self.read_file_data(file)?
            };

            // Write this file's data to the archive
            cursor.write_all(&data)?;
            current_offset += data.len() as u32;
        }

        // === STEP 2: Write the directory tree ===
        // This tells the game where each file is located
        let tree_start = cursor.position();

        // First, write how many files are in the archive
        cursor.write_u32::<LittleEndian>(self.files.len() as u32)?;

        // Then write information about each file
        for (i, file) in self.files.iter().enumerate() {
            let entry = Dat2FileEntry {
                filename_size: file.name.len() as u32,
                filename_bytes: file.name.as_bytes().to_vec(),
                compression_type: if file.compressed { 1 } else { 0 },
                real_size: file.size,          // Original file size
                packed_size: file.packed_size, // Compressed size (or same if not compressed)
                offset: file_offsets[i],       // Where this file starts in the archive
            };

            // Convert the entry to bytes and write it
            let entry_bytes = entry.to_bytes()?;
            cursor.write_all(&entry_bytes)?;
        }

        // === STEP 3: Write the footer ===
        // This helps the game find the directory tree
        let tree_end = cursor.position();
        let tree_size = (tree_end - tree_start) as u32; // Size of directory tree
        let total_size = tree_end + 8; // Total archive size (including this footer)

        let footer = Dat2Footer {
            tree_size,                   // How big is the directory tree?
            dat_size: total_size as u32, // How big is the entire file?
        };
        let footer_bytes = footer.to_bytes()?;
        cursor.write_all(&footer_bytes)?;

        // === STEP 4: Write everything to disk ===
        fs::write(path, output).context("Failed to write DAT2 file")?;

        Ok(())
    }
}
