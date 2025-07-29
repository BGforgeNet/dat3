/*!
# Common Types and Utilities

This module contains shared code that both DAT1 and DAT2 formats use.
It provides a single interface so the main program doesn't need to know
which DAT format it's working with.
*/

// Import the libraries we need
use anyhow::{bail, Context, Result}; // For error handling
use encoding_rs::WINDOWS_1252; // For old Windows text encoding
use std::fs; // File system operations
use std::path::{Path, PathBuf}; // Cross-platform path handling

/// Type-safe compression level (0-9)
#[derive(Debug, Clone, Copy)]
pub struct CompressionLevel(u8);

impl CompressionLevel {
    /// Create a new compression level (0=none, 9=maximum)
    pub fn new(level: u8) -> Result<Self> {
        if level <= 9 {
            Ok(Self(level))
        } else {
            bail!("Compression level must be 0-9, got {}", level)
        }
    }

    /// Get the raw compression level value
    pub fn level(&self) -> u8 {
        self.0
    }
}

// Our DAT format implementations
use crate::dat1::Dat1Archive; // Fallout 1 format
use crate::dat2::Dat2Archive; // Fallout 2 format

/// Represents a single file stored in a DAT archive
///
/// This structure contains all the metadata and optional data needed to
/// work with files in both DAT1 and DAT2 formats. It handles both files
/// that are already in an archive (with offset) and new files being added.
///
/// ## Fields Explanation
/// - **name**: File path using forward slashes (normalized format)
/// - **offset**: Byte position in the archive (0 for new files)
/// - **size**: Original file size before compression
/// - **packed_size**: Size after compression (equals size if not compressed)
/// - **compressed**: Whether compression was applied
/// - **data**: Raw file content (present for new/modified files)
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// File path with forward slashes (e.g., "ART/CRITTERS/FILE.FRM")
    pub name: String,
    /// Byte position where file data starts in the archive
    pub offset: u64,
    /// Original (uncompressed) file size in bytes
    pub size: u32,
    /// Compressed file size in bytes (equals size if not compressed)
    pub packed_size: u32,
    /// True if the file data is compressed
    pub compressed: bool,
    /// Raw file data for new/modified files (None for existing archive files)
    pub data: Option<Vec<u8>>,
}

impl FileEntry {
    /// Create a new file entry with data (compressed or uncompressed)
    ///
    /// This is used when adding files to an archive. The offset will be
    /// set later when the archive is saved.
    ///
    /// # Arguments
    /// * `name` - File path within the archive
    /// * `data` - File content (raw or already compressed)
    /// * `compressed` - Whether the data is compressed
    pub fn with_data(name: String, data: Vec<u8>, compressed: bool) -> Self {
        let packed_size = data.len() as u32;
        Self {
            name,
            offset: 0, // Will be set when writing to archive
            size: 0,   // Will be set by caller based on compression status
            packed_size,
            compressed,
            data: Some(data),
        }
    }

    /// Create a new file entry for a compressed file
    ///
    /// This method properly tracks both the original and compressed sizes,
    /// which is essential for DAT2 format compliance.
    ///
    /// # Arguments
    /// * `name` - File path within the archive
    /// * `original_data` - Uncompressed file content (for size calculation)
    /// * `compressed_data` - Compressed file content (what gets stored)
    pub fn with_compression_data(
        name: String,
        original_data: Vec<u8>,
        compressed_data: Vec<u8>,
    ) -> Self {
        Self {
            name,
            offset: 0,                                 // Will be set when writing
            size: original_data.len() as u32,          // Original file size
            packed_size: compressed_data.len() as u32, // Compressed size
            compressed: true,
            data: Some(compressed_data), // Store the compressed data
        }
    }
}

/// Unified interface for both DAT1 and DAT2 archives
///
/// This enum provides a single API for working with both Fallout archive formats.
/// The format is automatically detected when opening existing archives, and you
/// can explicitly choose the format when creating new ones.
///
/// ## Format Detection
///
/// When opening archives, the format is detected by examining the file structure:
/// - **DAT1**: Detected by reading the header and verifying the format
/// - **DAT2**: Used as fallback when DAT1 detection fails  
///
/// ## Usage
///
/// ```ignore
/// // Open existing archive (auto-detects format)
/// let archive = DatArchive::open("master.dat")?;
///
/// // Create new archives
/// let dat1_archive = DatArchive::new_dat1();
/// let dat2_archive = DatArchive::new_dat2();
/// ```
pub enum DatArchive {
    /// Fallout 1 format (hierarchical directories, LZSS compression)
    Dat1(Dat1Archive),
    /// Fallout 2 format (flat file list, zlib compression)
    Dat2(Dat2Archive),
}

impl DatArchive {
    /// Open an existing DAT archive, auto-detecting the format
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = fs::read(&path)
            .with_context(|| format!("Failed to read DAT file: {}", path.as_ref().display()))?;

        // Try to detect format by examining file structure
        if Self::is_dat1_format(&data) {
            Ok(DatArchive::Dat1(Dat1Archive::from_bytes(data)?))
        } else {
            Ok(DatArchive::Dat2(Dat2Archive::from_bytes(data)?))
        }
    }

    /// Create a new DAT1 archive
    pub fn new_dat1() -> Self {
        DatArchive::Dat1(Dat1Archive::new())
    }

    /// Create a new DAT2 archive
    pub fn new_dat2() -> Self {
        DatArchive::Dat2(Dat2Archive::new())
    }

    /// Detect if data is DAT1 format by examining the file structure
    /// DAT1 (Fallout 1) uses big-endian integers, DAT2 (Fallout 2) uses little-endian
    fn is_dat1_format(data: &[u8]) -> bool {
        // Need at least 16 bytes to check the header
        if data.len() < 16 {
            return false;
        }

        // Read the first few bytes as big-endian (DAT1 format)
        use byteorder::{BigEndian, ReadBytesExt};
        let mut cursor = std::io::Cursor::new(data);

        // Try to read directory count and format identifier
        if let Ok(dir_count) = cursor.read_u32::<BigEndian>() {
            if let Ok(format_id) = cursor.read_u32::<BigEndian>() {
                // DAT1 has reasonable directory counts (typically 1-50)
                // and specific format identifiers (0x0A or 0x5E)
                return dir_count > 0
                    && dir_count < 1000
                    && (format_id == 0x0A || format_id == 0x5E);
            }
        }

        // If we can't parse as DAT1, assume it's DAT2
        false
    }

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, files: &[String]) -> Result<()> {
        match self {
            DatArchive::Dat1(archive) => archive.list(files),
            DatArchive::Dat2(archive) => archive.list(files),
        }
    }

    /// Extract files from the archive
    pub fn extract<P: AsRef<Path>>(
        &self,
        output_dir: P,
        files: &[String],
        flat: bool,
    ) -> Result<()> {
        match self {
            DatArchive::Dat1(archive) => archive.extract(output_dir, files, flat),
            DatArchive::Dat2(archive) => archive.extract(output_dir, files, flat),
        }
    }

    /// Add a file to the archive
    pub fn add_file<P: AsRef<Path>>(
        &mut self,
        file_path: P,
        recursive: bool,
        compression: CompressionLevel,
        target_dir: Option<&str>,
    ) -> Result<()> {
        match self {
            DatArchive::Dat1(archive) => {
                archive.add_file(file_path, recursive, compression, target_dir)
            }
            DatArchive::Dat2(archive) => {
                archive.add_file(file_path, recursive, compression, target_dir)
            }
        }
    }

    /// Delete a file from the archive
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        match self {
            DatArchive::Dat1(archive) => archive.delete_file(file_name),
            DatArchive::Dat2(archive) => archive.delete_file(file_name),
        }
    }

    /// Save the archive to a file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        match self {
            DatArchive::Dat1(archive) => archive.save(path),
            DatArchive::Dat2(archive) => archive.save(path),
        }
    }
}

/// Pattern matching and filtering utilities
pub fn filter_and_track_patterns<'a, T>(
    items: &'a [T],
    patterns: &[String],
    matcher: impl Fn(&T, &str) -> bool,
) -> (Vec<&'a T>, Vec<String>) {
    if patterns.is_empty() {
        return (items.iter().collect(), Vec::new());
    }

    let mut patterns_found = vec![false; patterns.len()];
    let mut filtered_items = Vec::new();

    for item in items {
        for (idx, pattern) in patterns.iter().enumerate() {
            if matcher(item, pattern) {
                patterns_found[idx] = true;
                filtered_items.push(item);
                break; // Don't list the same item multiple times
            }
        }
    }

    let missing_patterns: Vec<String> = patterns
        .iter()
        .enumerate()
        .filter_map(|(idx, pattern)| {
            if !patterns_found[idx] {
                Some(pattern.clone())
            } else {
                None
            }
        })
        .collect();

    (filtered_items, missing_patterns)
}

/// Helper functions for common file and path operations
pub mod utils {
    use super::*;
    use std::borrow::Cow;

    /// Collect all files from a path (file or directory)
    /// If path is a file, returns just that file
    /// If path is a directory, returns all files in it (and subdirectories if recursive=true)
    pub fn collect_files<P: AsRef<Path>>(path: P, recursive: bool) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let path = path.as_ref();

        if path.is_file() {
            // If it's a single file, just add it
            files.push(path.to_path_buf());
        } else if path.is_dir() {
            // If it's a directory, scan through all entries
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();

                if entry_path.is_file() {
                    files.push(entry_path);
                } else if entry_path.is_dir() && recursive {
                    // If recursive is enabled, dive into subdirectories
                    files.extend(collect_files(&entry_path, recursive)?);
                }
            }
        }

        Ok(files)
    }

    /// Create all parent directories for a file path if they don't exist
    /// For example, if path is "dir1/dir2/file.txt", creates "dir1" and "dir1/dir2"
    pub fn ensure_dir_exists<P: AsRef<Path>>(path: P) -> Result<()> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
        Ok(())
    }

    /// Convert internal path format (forward slashes) to native OS format for display
    ///
    /// Internal representation always uses forward slashes for cross-platform code.
    /// This converts to the user's expected format:
    /// - Windows: forward slashes → backslashes
    /// - Unix/Linux: already forward slashes (no change)
    pub fn normalize_path_for_display(path: &str) -> String {
        #[cfg(windows)]
        {
            // Convert forward slashes to backslashes for Windows users
            path.replace('/', "\\")
        }
        #[cfg(not(windows))]
        {
            // Unix/Linux already uses forward slashes internally
            path.replace('\\', "/")
        }
    }

    /// Normalize user input path to internal format (forward slashes)
    ///
    /// Accepts both forward and backward slashes from users on any platform,
    /// converting them to our internal forward slash format for consistent matching.
    /// Uses Cow to avoid unnecessary allocations when no conversion is needed.
    pub fn normalize_user_path(path: &str) -> Cow<str> {
        if path.contains('\\') {
            Cow::Owned(path.replace('\\', "/"))
        } else {
            Cow::Borrowed(path)
        }
    }

    /// Expand @response-file syntax into actual file list
    ///
    /// If files contains exactly one item starting with '@', reads that file
    /// and returns its lines as the file list. Otherwise returns files as-is.
    ///
    /// # Arguments
    /// * `files` - Command line file arguments (may contain @response-file)
    ///
    /// # Returns
    /// * Expanded file list or error if response file cannot be read
    pub fn expand_response_files(files: &[String]) -> Result<Vec<String>> {
        // Check if we have exactly one argument starting with '@'
        if files.len() == 1 && files[0].starts_with('@') {
            let response_file = &files[0][1..]; // Remove '@' prefix
            let content = fs::read_to_string(response_file)
                .with_context(|| format!("Failed to read response file: {response_file}"))?;

            // Split by lines and filter out empty lines and comments
            let expanded_files: Vec<String> = content
                .lines()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(|line| line.to_string())
                .collect();

            Ok(expanded_files)
        } else if files.iter().any(|f| f.starts_with('@')) {
            // Mixed usage - response file with other arguments
            bail!("Cannot mix @response-file with explicit file arguments");
        } else {
            // No response file, return as-is
            Ok(files.to_vec())
        }
    }

    /// Convert any path to use backslashes (\) for DAT archive storage
    /// DAT files always store paths with backslashes
    pub fn normalize_path_for_archive(path: &str) -> String {
        path.replace('/', "\\")
    }

    /// Convert a DAT archive path to the current system's path format
    /// Archive paths use backslashes, convert to system separator for file operations
    pub fn to_system_path(dat_path: &str) -> PathBuf {
        PathBuf::from(dat_path.replace('\\', std::path::MAIN_SEPARATOR_STR))
    }

    /// Get just the filename (basename) from a path
    ///
    /// Works with both internal format (forward slashes) and archive format (backslashes).
    /// This is used for flat extraction where we want just the filename without directories.
    pub fn get_filename_from_dat_path(path: &str) -> &str {
        // Find the last path separator (try both forward and backward slashes)
        let last_forward = path.rfind('/');
        let last_backward = path.rfind('\\');

        let last_separator = match (last_forward, last_backward) {
            (Some(f), Some(b)) => Some(f.max(b)), // Use the rightmost separator
            (Some(f), None) => Some(f),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };

        if let Some(pos) = last_separator {
            &path[pos + 1..]
        } else {
            // No separator found, return the whole string
            path
        }
    }

    /// Convert filename bytes from old DAT files to modern UTF-8 strings
    ///
    /// Old Fallout games used Windows-1252 encoding, which includes special characters
    /// that aren't valid UTF-8. This function tries multiple approaches:
    /// 1. First try UTF-8 (for newer files)
    /// 2. Then try Windows-1252 (for old game files)  
    /// 3. Finally do a lossy conversion as last resort
    pub fn decode_filename(bytes: &[u8]) -> Result<String> {
        // Remove null bytes (C-style string terminators) from the end
        let trimmed_bytes = bytes
            .iter()
            .take_while(|&&b| b != 0) // Stop at first null byte
            .cloned()
            .collect::<Vec<u8>>();

        // Try UTF-8 first (most common case for newer files)
        if let Ok(utf8_str) = std::str::from_utf8(&trimmed_bytes) {
            return Ok(utf8_str.to_string());
        }

        // If UTF-8 fails, try Windows-1252 (legacy Windows encoding)
        let (decoded, _, had_errors) = WINDOWS_1252.decode(&trimmed_bytes);
        if had_errors {
            // Last resort: convert with replacement characters for invalid bytes
            let lossy_str = String::from_utf8_lossy(&trimmed_bytes);
            Ok(lossy_str.to_string())
        } else {
            Ok(decoded.to_string())
        }
    }
}
