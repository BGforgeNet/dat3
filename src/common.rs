/*!
# Common Types and Utilities

This module contains shared code that both DAT1 and DAT2 formats use.
It provides a single interface so the main program doesn't need to know
which DAT format it's working with.
*/

// Import the libraries we need
use anyhow::{bail, Context, Result}; // For error handling
use glob::glob; // Cross-platform glob expansion
use std::fs; // File system operations
use std::io::{self, Write}; // For stdout handling
use std::path::{Path, PathBuf}; // Cross-platform path handling

/// Write to stdout, exiting cleanly on broken pipe (e.g., when piped to `head`)
fn print_stdout(args: std::fmt::Arguments) {
    if writeln!(io::stdout(), "{args}").is_err() {
        // Broken pipe or other write error - exit cleanly
        std::process::exit(0);
    }
}

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

// DAT format detection constants
const DAT1_FORMAT_ID_1: u32 = 0x0A;
const DAT1_FORMAT_ID_2: u32 = 0x5E;
const DAT1_MAX_DIRECTORIES: u32 = 1000;

/// Represents a single file stored in a DAT archive
///
/// This structure contains all the metadata and optional data needed to
/// work with files in both DAT1 and DAT2 formats. It handles both files
/// that are already in an archive (with offset) and new files being added.
///
/// ## Fields Explanation
/// - **name**: File path using backslashes (DAT archive format)
/// - **offset**: Byte position in the archive (0 for new files)
/// - **size**: Original file size before compression
/// - **packed_size**: Size after compression (equals size if not compressed)
/// - **compressed**: Whether compression was applied
/// - **data**: Raw file content (present for new/modified files)
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// File path with backslashes (e.g., "ART\\CRITTERS\\FILE.FRM")
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

impl AsRef<FileEntry> for FileEntry {
    fn as_ref(&self) -> &FileEntry {
        self
    }
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

/// Extraction mode for archive files
///
/// This enum controls how files are extracted from archives:
/// - **PreserveStructure**: Maintains the original directory structure
/// - **Flat**: Extracts all files to the output directory without subdirectories
#[derive(Debug, Clone, Copy)]
pub enum ExtractionMode {
    /// Preserve the original directory structure when extracting
    PreserveStructure,
    /// Extract all files to a flat directory structure (no subdirectories)
    Flat,
}

/// Common interface for DAT archive operations
///
/// This trait defines the operations that both DAT1 and DAT2 formats support.
/// It allows uniform handling of archives regardless of their format.
pub trait ArchiveFormat {
    /// List files in the archive (all or filtered by patterns)
    fn list(&self, files: &[String]) -> Result<()>;

    /// Extract files from the archive
    fn extract(&self, output_dir: &Path, files: &[String], mode: ExtractionMode) -> Result<()>;

    /// Add a file to the archive
    fn add_file(
        &mut self,
        file_path: &Path,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        strip_leading_directory: bool,
    ) -> Result<()>;

    /// Delete a file from the archive
    fn delete_file(&mut self, file_name: &str) -> Result<()>;

    /// Save the archive to a file
    fn save(&self, path: &Path) -> Result<()>;
}

/// Unified interface for both DAT1 and DAT2 archives
///
/// This wrapper provides a single API for working with both Fallout archive formats.
/// The format is automatically detected when opening existing archives, and you
/// can explicitly choose the format when creating new ones.
///
/// ## Memory Usage
///
/// **Important**: The entire archive is loaded into memory when opened. This works
/// well for typical Fallout archives (up to ~200MB), but may not scale to very
/// large files. For archives significantly larger than available RAM, consider
/// implementing streaming I/O.
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
pub struct DatArchive {
    inner: Box<dyn ArchiveFormat>,
    format: ArchiveFormatType,
}

/// The detected format of a DAT archive
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormatType {
    /// Fallout 1 format (hierarchical directories, LZSS compression)
    Dat1,
    /// Fallout 2 format (flat file list, zlib compression)
    Dat2,
}

impl DatArchive {
    /// Open an existing DAT archive, auto-detecting the format
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = fs::read(&path)
            .with_context(|| format!("Failed to read DAT file: {}", path.as_ref().display()))?;

        // Try to detect format by examining file structure
        if Self::is_dat1_format(&data) {
            Ok(Self {
                inner: Box::new(Dat1Archive::from_bytes(data)?),
                format: ArchiveFormatType::Dat1,
            })
        } else {
            Ok(Self {
                inner: Box::new(Dat2Archive::from_bytes(data)?),
                format: ArchiveFormatType::Dat2,
            })
        }
    }

    /// Create a new DAT1 archive
    pub fn new_dat1() -> Self {
        Self {
            inner: Box::new(Dat1Archive::new()),
            format: ArchiveFormatType::Dat1,
        }
    }

    /// Create a new DAT2 archive
    pub fn new_dat2() -> Self {
        Self {
            inner: Box::new(Dat2Archive::new()),
            format: ArchiveFormatType::Dat2,
        }
    }

    /// Check if this is a DAT1 archive
    pub fn is_dat1(&self) -> bool {
        self.format == ArchiveFormatType::Dat1
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
                    && dir_count < DAT1_MAX_DIRECTORIES
                    && (format_id == DAT1_FORMAT_ID_1 || format_id == DAT1_FORMAT_ID_2);
            }
        }

        // If we can't parse as DAT1, assume it's DAT2
        false
    }

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, files: &[String]) -> Result<()> {
        self.inner.list(files)
    }

    /// Extract files from the archive
    pub fn extract<P: AsRef<Path>>(
        &self,
        output_dir: P,
        files: &[String],
        mode: ExtractionMode,
    ) -> Result<()> {
        self.inner.extract(output_dir.as_ref(), files, mode)
    }

    /// Add a file to the archive (directories are processed recursively)
    pub fn add_file<P: AsRef<Path>>(
        &mut self,
        file_path: P,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        strip_leading_directory: bool,
    ) -> Result<()> {
        self.inner.add_file(
            file_path.as_ref(),
            compression,
            target_dir,
            strip_leading_directory,
        )
    }

    /// Delete a file from the archive
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        self.inner.delete_file(file_name)
    }

    /// Save the archive to a file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        self.inner.save(path.as_ref())
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

    /// Result of expanding file patterns
    #[derive(Debug, Clone)]
    pub struct ExpandedFiles {
        /// The expanded file paths
        pub paths: Vec<PathBuf>,
        /// Flags indicating whether each path should have its leading directory stripped
        pub strip_directory_flags: Vec<bool>,
    }

    impl ExpandedFiles {
        /// Create a new ExpandedFiles result from PathBufs
        pub fn new(paths: Vec<PathBuf>, strip_directory_flags: Vec<bool>) -> Self {
            debug_assert_eq!(
                paths.len(),
                strip_directory_flags.len(),
                "Paths and flags must have the same length"
            );
            Self {
                paths,
                strip_directory_flags,
            }
        }

        /// Convert into an iterator that consumes the struct, yielding (PathBuf, bool)
        pub fn into_iter(self) -> impl Iterator<Item = (PathBuf, bool)> {
            self.paths.into_iter().zip(self.strip_directory_flags)
        }
    }

    /// Print formatted file listing to stdout
    /// Common implementation used by both DAT1 and DAT2 formats
    /// Exits cleanly on broken pipe (e.g., when piped to `head`)
    pub fn print_file_listing<T: AsRef<FileEntry>>(files: &[T]) {
        print_stdout(format_args!(
            "{:>11} {:>11}  {:>4}  Name",
            "Size", "Packed", "Comp"
        ));
        print_stdout(format_args!("{}", "-".repeat(50)));

        for file in files {
            let file = file.as_ref();
            let comp_str = if file.compressed { "Yes" } else { "No" };
            let display_name = normalize_path_for_display(&file.name);
            print_stdout(format_args!(
                "{:>11} {:>11}  {:>4}  {}",
                file.size, file.packed_size, comp_str, display_name
            ));
        }
    }

    /// Collect all files from a path (file or directory)
    /// If path is a file, returns just that file
    /// If path is a directory, returns all files in it and all subdirectories recursively
    /// Validates that all filenames are ASCII-only before returning
    pub fn collect_files<P: AsRef<Path>>(path: P) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let path = path.as_ref();

        // Check if the path exists
        if !path.exists() {
            bail!("Path does not exist: {}", path.display());
        }

        if path.is_file() {
            // If it's a single file, just add it
            files.push(path.to_path_buf());
        } else if path.is_dir() {
            // If it's a directory, scan through all entries recursively
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();

                if entry_path.is_file() {
                    files.push(entry_path);
                } else if entry_path.is_dir() {
                    // Always dive into subdirectories
                    files.extend(collect_files(&entry_path)?);
                }
            }
        }

        // Validate all file paths are ASCII-only before returning
        for file in &files {
            if let Some(path_str) = file.to_str() {
                validate_filename_ascii(path_str)
                    .with_context(|| format!("Invalid path: {}", file.display()))?;
            } else {
                bail!("Invalid path encoding: {}", file.display());
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

    /// Convert internal path format (backslashes) to native OS format for display
    ///
    /// Internal representation uses backslashes (DAT archive format).
    /// This converts to the user's expected format:
    /// - Windows: already backslashes (no change needed)
    /// - Unix/Linux: backslashes → forward slashes
    pub fn normalize_path_for_display(path: &str) -> String {
        #[cfg(windows)]
        {
            path.to_string()
        }
        #[cfg(not(windows))]
        {
            path.replace('\\', "/")
        }
    }

    /// Normalize user input path to internal format (backslashes)
    ///
    /// Accepts both forward and backward slashes from users on any platform,
    /// converting them to our internal backslash format for consistent matching.
    /// Uses Cow to avoid unnecessary allocations when no conversion is needed.
    pub fn normalize_user_path(path: &str) -> Cow<'_, str> {
        if path.contains('/') {
            Cow::Owned(path.replace('/', "\\"))
        } else {
            Cow::Borrowed(path)
        }
    }

    /// Normalize a collection of user input patterns to internal format
    ///
    /// This is a common pattern used throughout the codebase for normalizing
    /// user-provided file patterns before filtering operations.
    pub fn normalize_user_patterns(patterns: &[String]) -> Vec<String> {
        patterns
            .iter()
            .map(|p| normalize_user_path(p).into_owned())
            .collect()
    }

    /// Check if a string contains glob metacharacters
    ///
    /// This is more robust than checking individual characters and can be
    /// extended to handle escaped characters in the future.
    fn contains_glob_metacharacters(pattern: &str) -> bool {
        pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
    }

    /// Check if a pattern indicates directory stripping (starts with ./ or .\)
    fn should_strip_directory(pattern: &str) -> bool {
        pattern.starts_with("./") || pattern.starts_with(".\\")
    }

    /// Normalize a glob pattern for cross-platform use
    ///
    /// Converts backslashes to forward slashes for the glob library.
    /// Handles escaped backslashes correctly.
    fn normalize_glob_pattern(pattern: &str) -> String {
        pattern
            .replace("\\\\", "\x00") // Temporarily replace escaped backslashes
            .replace('\\', "/") // Convert path separators
            .replace('\x00', "\\") // Restore escaped backslashes
    }

    /// Expand @response-file syntax and glob patterns into actual file list
    ///
    /// If files contains exactly one item starting with '@', reads that file
    /// and returns its lines as the file list. Otherwise, expands any glob
    /// patterns and returns the expanded file list.
    ///
    /// # Arguments
    /// * `files` - Command line file arguments (may contain @response-file or globs)
    ///
    /// # Returns
    /// * Expanded file list or error if response file cannot be read or pattern fails
    ///
    /// # Example
    /// ```ignore
    /// let files = vec!["*.txt".to_string()];
    /// let expanded = expand_response_files(&files)?;
    /// // expanded contains all .txt files in current directory as PathBuf
    /// ```
    pub fn expand_response_files(files: &[String]) -> Result<Vec<PathBuf>> {
        Ok(expand_response_files_with_stripping(files)?.paths)
    }

    /// Expand @response-file syntax and glob patterns, tracking directory stripping per file
    ///
    /// Returns both the expanded file list and flags indicating which files need directory stripping.
    /// Directory stripping is applied only to files that come from patterns starting with "./" or ".\"
    ///
    /// # Arguments
    /// * `files` - Command line file arguments (may contain @response-file or globs)
    ///
    /// # Returns
    /// * `ExpandedFiles` struct containing paths and strip flags
    ///
    /// # Example
    /// ```ignore
    /// let files = vec!["./src/*.rs".to_string()];
    /// let expanded = expand_response_files_with_stripping(&files)?;
    /// // All files from ./src/*.rs will have strip_directory_flags set to true
    /// ```
    pub fn expand_response_files_with_stripping(files: &[String]) -> Result<ExpandedFiles> {
        // Handle response file case
        if files.len() == 1 && files[0].starts_with('@') {
            return expand_response_file(&files[0][1..]);
        }

        // Check for mixed usage
        if files.iter().any(|f| f.starts_with('@')) {
            bail!("Cannot mix @response-file with explicit file arguments");
        }

        // Process regular files and glob patterns
        expand_file_patterns(files)
    }

    /// Expand a response file into a list of files
    fn expand_response_file(response_file_path: &str) -> Result<ExpandedFiles> {
        let content = fs::read_to_string(response_file_path)
            .with_context(|| format!("Failed to read response file: {response_file_path}"))?;

        let paths: Vec<PathBuf> = content
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(PathBuf::from)
            .collect();

        // Response files don't use directory stripping
        let strip_flags = vec![false; paths.len()];
        Ok(ExpandedFiles::new(paths, strip_flags))
    }

    /// Expand file patterns (including globs) into actual file paths
    fn expand_file_patterns(patterns: &[String]) -> Result<ExpandedFiles> {
        let mut paths = Vec::new();
        let mut strip_flags = Vec::new();

        for pattern in patterns {
            let should_strip = should_strip_directory(pattern);

            if contains_glob_metacharacters(pattern) {
                // Handle glob pattern
                let expanded = expand_single_glob(pattern, should_strip)?;
                paths.extend(expanded.paths);
                strip_flags.extend(expanded.strip_directory_flags);
            } else {
                // Regular file path - add as-is
                paths.push(PathBuf::from(pattern));
                strip_flags.push(should_strip);
            }
        }

        Ok(ExpandedFiles::new(paths, strip_flags))
    }

    /// Expand a single glob pattern
    fn expand_single_glob(pattern: &str, should_strip: bool) -> Result<ExpandedFiles> {
        let normalized_pattern = normalize_glob_pattern(pattern);
        let mut paths = Vec::new();
        let mut strip_flags = Vec::new();

        let glob_iter = glob(&normalized_pattern)
            .with_context(|| format!("Invalid glob pattern: {pattern}"))?;

        for entry in glob_iter {
            match entry {
                Ok(path) => {
                    paths.push(path);
                    strip_flags.push(should_strip);
                }
                Err(e) => {
                    bail!("Error expanding glob pattern '{}': {}", pattern, e);
                }
            }
        }

        if paths.is_empty() {
            bail!("No files found matching pattern: {}", pattern);
        }

        Ok(ExpandedFiles::new(paths, strip_flags))
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
    /// Accepts both forward and backward slashes for flexibility with user input.
    /// Internal storage uses backslashes (DAT archive format).
    pub fn get_filename_from_dat_path(path: &str) -> &str {
        // Find the last path separator (either forward or backward slash)
        path.rfind(['/', '\\'])
            .map(|pos| &path[pos + 1..])
            .unwrap_or(path)
    }

    /// Get the directory part from a DAT archive path.
    ///
    /// Returns "." if the path has no directory component.
    pub fn get_dirname_from_dat_path(path: &str) -> &str {
        path.rfind(['/', '\\'])
            .map(|pos| &path[..pos])
            .unwrap_or(".")
    }

    /// Convert filename bytes from DAT files to ASCII strings
    ///
    /// Strictly requires ASCII-only filenames. Fails if any non-ASCII characters are found.
    pub fn decode_filename(bytes: &[u8]) -> Result<String> {
        // Remove null bytes (C-style string terminators)
        let trimmed_bytes: Vec<u8> = bytes.iter().take_while(|&&b| b != 0).copied().collect();

        // Only accept strict ASCII
        match std::str::from_utf8(&trimmed_bytes) {
            Ok(ascii_str) => {
                validate_filename_ascii(ascii_str)?;
                Ok(ascii_str.to_string())
            }
            Err(_) => {
                bail!("Invalid filename encoding - not valid UTF-8")
            }
        }
    }

    /// Validate that a filename string contains only ASCII characters
    ///
    /// This is used both when decoding filenames from DAT files and when adding
    /// new files to archives to ensure ASCII-only policy compliance.
    pub fn validate_filename_ascii(filename: &str) -> Result<()> {
        if filename.is_ascii() {
            Ok(())
        } else {
            bail!("Non-ASCII filename found: {:?}", filename)
        }
    }

    /// Calculate the archive path for a file being added to a DAT archive
    ///
    /// This handles the complex logic of determining where a file should be placed
    /// in the archive based on the input file path, base path, and target directory.
    /// Supports 7z-style directory stripping when strip_leading_directory is true.
    pub fn calculate_archive_path(
        file: &std::path::Path,
        base_path: &std::path::Path,
        target_dir: Option<&str>,
        strip_leading_directory: bool,
    ) -> Result<String> {
        let archive_path = match target_dir {
            Some(target) => {
                if base_path.is_dir() {
                    let relative_path = if let Some(parent) = base_path.parent() {
                        file.strip_prefix(parent).unwrap_or(file).to_string_lossy()
                    } else {
                        file.to_string_lossy()
                    };
                    format!("{target}/{relative_path}")
                } else {
                    let filename = file
                        .file_name()
                        .ok_or_else(|| anyhow::anyhow!("Invalid filename for: {}", file.display()))?
                        .to_string_lossy();
                    format!("{target}/{filename}")
                }
            }
            None => {
                // Always preserve the full relative path as specified
                // This ensures consistent behavior whether files were specified
                // individually or as part of a directory expansion
                let mut path = file.to_string_lossy().into_owned();

                // Apply directory stripping if requested (7z behavior)
                if strip_leading_directory {
                    path = strip_leading_directory_from_path(&path);
                }

                path
            }
        };

        Ok(normalize_path_for_archive(&archive_path))
    }

    /// Normalize path separators and collapse consecutive slashes
    ///
    /// This is more efficient than repeatedly calling string.replace()
    /// and handles all normalization in a single pass.
    fn normalize_path_separators(path: &str) -> String {
        let mut result = String::with_capacity(path.len());
        let mut last_was_slash = false;

        for ch in path.chars() {
            match ch {
                '\\' | '/' => {
                    if !last_was_slash {
                        result.push('/');
                        last_was_slash = true;
                    }
                    // Skip consecutive slashes
                }
                _ => {
                    result.push(ch);
                    last_was_slash = false;
                }
            }
        }

        result
    }

    /// Strip the leading directory component from a path (7z-style behavior)
    ///
    /// Examples:
    /// - "patch000/file.txt" -> "file.txt"
    /// - "./patch000/file.txt" -> "file.txt"
    /// - "patch000/subdir/file.txt" -> "subdir/file.txt"
    /// - "file.txt" -> "file.txt" (no change)
    ///
    /// This implementation is more efficient than the previous one,
    /// using a single-pass algorithm instead of multiple string replacements.
    pub fn strip_leading_directory_from_path(path: &str) -> String {
        // Normalize path separators in a single pass
        let normalized = normalize_path_separators(path);

        // Remove ./ prefix if present
        let without_dot_slash = normalized.strip_prefix("./").unwrap_or(&normalized);

        // Find first separator and skip the leading directory
        match without_dot_slash.find('/') {
            Some(sep_pos) => without_dot_slash[sep_pos + 1..].to_string(),
            None => without_dot_slash.to_string(),
        }
    }
}
