/*!
# DAT2 Archive Format (Fallout 2)

Little-endian, flat file list, zlib compression, parallel extraction via rayon.

## File layout:
1. File data (all files concatenated)
2. Directory tree (file count + file entries)
3. Footer (8 bytes): tree_size + dat_size
*/

use anyhow::{bail, Context, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use deku::prelude::*;
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Instant;

use crate::common::{self, utils, CompressionLevel, ExtractionMode, FileEntry};

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

        // Directory tree position: dat_size - tree_size - 8 (footer)
        let tree_start = footer.dat_size as usize - footer.tree_size as usize - 8;
        if tree_start < 4 {
            bail!("Invalid directory tree position");
        }

        // Read file count
        let mut cursor = Cursor::new(&data[tree_start..]);
        let file_count = cursor
            .read_u32::<LittleEndian>()
            .context("Failed to read file count from DAT2 directory tree")?;

        // Parse file entries using deku
        let mut files = Vec::with_capacity(file_count as usize);
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
        let files_to_extract = common::filter_files_by_patterns(&self.files, files);
        self.extract_files_parallel(&files_to_extract, output_dir, mode)
    }

    /// Parallel extraction using rayon
    fn extract_files_parallel(
        &self,
        files_to_extract: &[&FileEntry],
        output_dir: &Path,
        mode: ExtractionMode,
    ) -> Result<()> {
        let archive_data = Arc::new(self.data.as_slice());
        let total_files = files_to_extract.len();
        let completed = Arc::new(AtomicUsize::new(0));

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

                // Read and optionally decompress
                let file_data = self.read_file_data_from_slice(&archive_data, file)?;
                let final_data = if file.compressed {
                    Self::decompress_zlib(&file_data, file.size as usize)
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

    /// Read file data from a shared byte slice (thread-safe for parallel extraction)
    fn read_file_data_from_slice(&self, archive_data: &[u8], file: &FileEntry) -> Result<Vec<u8>> {
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

    /// Read file data from the archive's own data buffer
    fn read_file_data(&self, file: &FileEntry) -> Result<Vec<u8>> {
        self.read_file_data_from_slice(&self.data, file)
    }

    /// Decompress zlib data with a pre-allocated output buffer
    fn decompress_zlib(data: &[u8], expected_size: usize) -> Result<Vec<u8>> {
        let mut decoder = ZlibDecoder::new(data);
        let mut decompressed = Vec::with_capacity(expected_size);
        decoder
            .read_to_end(&mut decompressed)
            .context("Failed to decompress zlib data")?;
        Ok(decompressed)
    }

    /// Compress data using zlib
    fn compress_zlib(data: &[u8], level: u8) -> Result<Vec<u8>> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level as u32));
        encoder.write_all(data)?;
        encoder.finish().context("Failed to compress with zlib")
    }

    /// Process a single file for adding to the archive
    fn process_single_file_for_adding(
        &self,
        file: &Path,
        base_path: &Path,
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
            let compressed_data = Self::compress_zlib(&data, compression.level())?;
            // Only use compression if it actually saves space
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

    /// Add files to the archive (directories processed recursively, parallel)
    pub fn add_file(
        &mut self,
        file_path: &Path,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        strip_leading_directory: bool,
    ) -> Result<()> {
        let base_path = file_path;
        let files = utils::collect_files(file_path).with_context(|| {
            format!(
                "Failed to collect files from path '{}'",
                file_path.display()
            )
        })?;

        // Process files in parallel
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

        let new_entries = results?; // Collect results, propagating the first error if any file failed

        // Remove existing files that match new file names
        let new_file_names: HashSet<String> = new_entries.iter().map(|e| e.name.clone()).collect();
        self.files
            .retain(|existing_file| !new_file_names.contains(&existing_file.name));

        // Add new files, deduplicating within the batch (keep first occurrence).
        // This can happen if the user passes the same file or two files with the same name.
        let mut seen_names = HashSet::new();
        for entry in new_entries {
            if seen_names.insert(entry.name.clone()) {
                self.files.push(entry);
            }
        }

        // DAT2 format requires files sorted alphabetically (case-insensitive)
        self.files
            .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        Ok(())
    }

    /// Delete a file from the archive by name
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        common::delete_file_from_list(&mut self.files, file_name)
    }

    /// Save the archive to a DAT2 file.
    ///
    /// DAT2 layout: file data, then directory tree, then 8-byte footer.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut output = Vec::new();
        let mut cursor = Cursor::new(&mut output);

        // Step 1: Write all file data
        let mut current_offset = 0u32;
        let mut file_offsets = Vec::new();

        for file in &self.files {
            file_offsets.push(current_offset);

            let data = if let Some(ref file_data) = file.data {
                file_data.clone() // File data is already in memory (newly added file)
            } else {
                self.read_file_data(file)? // Need to read from the original archive
            };

            cursor.write_all(&data)?;
            current_offset += data.len() as u32;
        }

        // Step 2: Write directory tree
        let tree_start = cursor.position();
        cursor.write_u32::<LittleEndian>(self.files.len() as u32)?;

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

        // Step 3: Write 8-byte footer
        let tree_end = cursor.position();
        let tree_size = (tree_end - tree_start) as u32;
        let total_size = tree_end + 8;

        let footer = Dat2Footer {
            tree_size,
            dat_size: total_size as u32,
        };
        let footer_bytes = footer.to_bytes()?;
        cursor.write_all(&footer_bytes)?;

        // Step 4: Write to disk
        fs::write(path, output).context("Failed to write DAT2 file")?;

        Ok(())
    }
}
