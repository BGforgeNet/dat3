/*!
# Common Types and Utilities

Shared code for both DAT1 and DAT2 formats. Provides a unified `DatArchive`
enum so callers don't need to know which format they're working with.
*/

use anyhow::{bail, Context, Result};
use glob::glob;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::dat1::Dat1Archive;
use crate::dat2::Dat2Archive;

// DAT1 format detection: big-endian header with known format IDs
const DAT1_FORMAT_ID_1: u32 = 0x0A;
const DAT1_FORMAT_ID_2: u32 = 0x5E;
const DAT1_MAX_DIRECTORIES: u32 = 1000;

/// Write to stdout, exiting cleanly on broken pipe (e.g., when piped to `head`)
fn print_stdout(args: std::fmt::Arguments) {
    if writeln!(io::stdout(), "{args}").is_err() {
        std::process::exit(0);
    }
}

// ── Core types ─────────────────────────────────────────────────────

/// Type-safe compression level (0-9).
///
/// Wraps a `u8` so invalid values are rejected at construction time
/// rather than causing errors deep in compression code.
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

/// Represents a single file stored in a DAT archive.
///
/// Used by both DAT1 and DAT2 formats. For files already in an archive,
/// `data` is None and content is read from the raw archive bytes using `offset`.
/// For newly added files, `data` holds the content and `offset` is 0.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// File path with backslashes (e.g., "ART\\CRITTERS\\FILE.FRM")
    pub name: String,
    /// Byte position where file data starts in the archive
    pub offset: u64,
    /// Original (uncompressed) file size in bytes
    pub size: u32,
    /// Compressed file size (equals `size` if not compressed)
    pub packed_size: u32,
    /// Whether the file data is compressed
    pub compressed: bool,
    /// Raw file data for new/modified files (None for existing archive files)
    pub data: Option<Vec<u8>>,
}

/// Allows `&[FileEntry]` to work with `print_file_listing`,
/// which accepts `&[T: AsRef<FileEntry>]` so it also works with `&[&FileEntry]`.
impl AsRef<FileEntry> for FileEntry {
    fn as_ref(&self) -> &FileEntry {
        self
    }
}

impl FileEntry {
    /// Create a file entry with uncompressed data.
    /// The `offset` is set to 0 and will be computed when saving.
    pub fn with_data(name: String, data: Vec<u8>, compressed: bool) -> Self {
        let packed_size = data.len() as u32;
        Self {
            name,
            offset: 0,
            size: 0, // Caller sets this based on compression status
            packed_size,
            compressed,
            data: Some(data),
        }
    }

    /// Create a file entry tracking both original and compressed sizes.
    /// Essential for DAT2 format where the directory tree stores both.
    pub fn with_compression_data(
        name: String,
        original_data: Vec<u8>,
        compressed_data: Vec<u8>,
    ) -> Self {
        Self {
            name,
            offset: 0,
            size: original_data.len() as u32,
            packed_size: compressed_data.len() as u32,
            compressed: true,
            data: Some(compressed_data),
        }
    }
}

/// Controls how files are extracted from archives
#[derive(Debug, Clone, Copy)]
pub enum ExtractionMode {
    /// Keep the original directory structure
    PreserveStructure,
    /// Put all files in one flat directory
    Flat,
}

// ── DatArchive enum ────────────────────────────────────────────────

/// Unified interface for both DAT1 and DAT2 archives.
///
/// Uses an enum instead of trait objects because there are exactly two
/// known formats - this gives us static dispatch, exhaustive matching,
/// and no heap allocation for the wrapper.
///
/// **Memory**: The entire archive is loaded into memory on open. This works
/// well for typical Fallout archives (up to ~200MB).
///
/// ```ignore
/// let archive = DatArchive::open("master.dat")?;  // auto-detects format
/// let dat1 = DatArchive::new_dat1();               // create new DAT1
/// let dat2 = DatArchive::new_dat2();               // create new DAT2
/// ```
pub enum DatArchive {
    /// Fallout 1 format (big-endian, hierarchical dirs, LZSS compression)
    Dat1(Dat1Archive),
    /// Fallout 2 format (little-endian, flat file list, zlib compression)
    Dat2(Dat2Archive),
}

impl DatArchive {
    /// Open an existing DAT archive, auto-detecting the format
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = fs::read(&path)
            .with_context(|| format!("Failed to read DAT file: {}", path.as_ref().display()))?;

        if Self::is_dat1_format(&data) {
            Ok(Self::Dat1(Dat1Archive::from_bytes(data)?))
        } else {
            Ok(Self::Dat2(Dat2Archive::from_bytes(data)?))
        }
    }

    /// Create a new empty DAT1 (Fallout 1) archive
    pub fn new_dat1() -> Self {
        Self::Dat1(Dat1Archive::new())
    }

    /// Create a new empty DAT2 (Fallout 2) archive
    pub fn new_dat2() -> Self {
        Self::Dat2(Dat2Archive::new())
    }

    /// Check if this is a DAT1 archive
    pub fn is_dat1(&self) -> bool {
        matches!(self, Self::Dat1(_))
    }

    /// Detect DAT1 format by examining the big-endian header.
    /// DAT1 has a directory count and a known format identifier (0x0A or 0x5E).
    fn is_dat1_format(data: &[u8]) -> bool {
        if data.len() < 16 {
            return false;
        }

        use byteorder::{BigEndian, ReadBytesExt};
        let mut cursor = std::io::Cursor::new(data);

        if let Ok(dir_count) = cursor.read_u32::<BigEndian>() {
            if let Ok(format_id) = cursor.read_u32::<BigEndian>() {
                // DAT1 has reasonable directory counts (typically 1-50)
                // and specific format identifiers
                return dir_count > 0
                    && dir_count < DAT1_MAX_DIRECTORIES
                    && (format_id == DAT1_FORMAT_ID_1 || format_id == DAT1_FORMAT_ID_2);
            }
        }

        false
    }

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, files: &[String]) -> Result<()> {
        match self {
            Self::Dat1(a) => a.list(files),
            Self::Dat2(a) => a.list(files),
        }
    }

    /// Extract files from the archive
    pub fn extract<P: AsRef<Path>>(
        &self,
        output_dir: P,
        files: &[String],
        mode: ExtractionMode,
    ) -> Result<()> {
        match self {
            Self::Dat1(a) => a.extract(output_dir.as_ref(), files, mode),
            Self::Dat2(a) => a.extract(output_dir.as_ref(), files, mode),
        }
    }

    /// Add a file to the archive (directories are processed recursively)
    pub fn add_file<P: AsRef<Path>>(
        &mut self,
        file_path: P,
        compression: CompressionLevel,
        target_dir: Option<&str>,
    ) -> Result<()> {
        match self {
            Self::Dat1(a) => a.add_file(file_path.as_ref(), compression, target_dir),
            Self::Dat2(a) => a.add_file(file_path.as_ref(), compression, target_dir),
        }
    }

    /// Delete a file from the archive
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        match self {
            Self::Dat1(a) => a.delete_file(file_name),
            Self::Dat2(a) => a.delete_file(file_name),
        }
    }

    /// Save the archive to a file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        match self {
            Self::Dat1(a) => a.save(path.as_ref()),
            Self::Dat2(a) => a.save(path.as_ref()),
        }
    }
}

// ── Shared archive operations ──────────────────────────────────────

/// List files using shared filter-and-print logic.
///
/// Both DAT1 and DAT2 use this same flow:
/// normalize patterns -> filter entries -> print listing -> report missing.
pub fn list_files_filtered(all_files: &[&FileEntry], patterns: &[String]) -> Result<()> {
    let normalized_patterns = utils::normalize_user_patterns(patterns);

    let (files_to_list, missing_patterns) =
        filter_and_track_patterns(all_files, &normalized_patterns, |file, pattern| {
            utils::matches_pattern(&file.name, pattern)
        });

    utils::print_file_listing(&files_to_list);

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

/// Filter files by patterns and return matched files.
///
/// Shared by DAT1 and DAT2 extract paths.
pub fn filter_files_by_patterns<'a>(
    all_files: &'a [FileEntry],
    patterns: &[String],
) -> Vec<&'a FileEntry> {
    let normalized_patterns = utils::normalize_user_patterns(patterns);

    let (filtered, _) =
        filter_and_track_patterns(all_files, &normalized_patterns, |file, pattern| {
            utils::matches_pattern(&file.name, pattern)
        });

    filtered
}

/// Delete a file from a list by normalized name.
///
/// Shared by DAT1 and DAT2 delete implementations.
pub fn delete_file_from_list(files: &mut Vec<FileEntry>, file_name: &str) -> Result<()> {
    let normalized_name = utils::normalize_user_path(file_name).into_owned();

    if let Some(pos) = files.iter().position(|f| f.name == normalized_name) {
        let display_name = utils::normalize_path_for_display(&normalized_name);
        println!("Deleting: {display_name}");
        files.remove(pos);
        Ok(())
    } else {
        bail!(
            "File not found: {}",
            utils::normalize_path_for_display(file_name)
        );
    }
}

/// Filter items by patterns, tracking which patterns matched.
///
/// Returns (matched_items, unmatched_patterns). Each item is matched at most
/// once (by the first matching pattern) to avoid duplicates in listings.
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
                break; // Don't add the same item twice if multiple patterns match it
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

// ── Utility functions ──────────────────────────────────────────────

/// Helper functions for file/path operations and pattern matching
pub mod utils {
    use super::*;
    use std::borrow::Cow;

    /// Print formatted file listing to stdout.
    /// Exits cleanly on broken pipe (e.g., when piped to `head`).
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

    /// Collect all files from a path (file or directory, recursive).
    /// Validates that all filenames are ASCII-only.
    pub fn collect_files<P: AsRef<Path>>(path: P) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let path = path.as_ref();

        if !path.exists() {
            bail!("Path does not exist: {}", path.display());
        }

        if path.is_file() {
            files.push(path.to_path_buf());
        } else if path.is_dir() {
            // Scan directory recursively
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();

                if entry_path.is_file() {
                    files.push(entry_path);
                } else if entry_path.is_dir() {
                    // Always recurse into subdirectories
                    files.extend(collect_files(&entry_path)?);
                }
            }
        }

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

    /// Create all parent directories for a file path
    pub fn ensure_dir_exists<P: AsRef<Path>>(path: P) -> Result<()> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
        }
        Ok(())
    }

    /// Convert internal backslash paths to OS-native format for display.
    /// On Unix this converts `\` to `/`; on Windows it's a no-op.
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

    /// Normalize user input path to internal backslash format.
    /// Uses `Cow` to avoid allocation when the path already uses backslashes.
    pub fn normalize_user_path(path: &str) -> Cow<'_, str> {
        if path.contains('/') {
            Cow::Owned(path.replace('/', "\\"))
        } else {
            Cow::Borrowed(path)
        }
    }

    /// Normalize a batch of user patterns to internal backslash format
    pub fn normalize_user_patterns(patterns: &[String]) -> Vec<String> {
        patterns
            .iter()
            .map(|p| normalize_user_path(p).into_owned())
            .collect()
    }

    /// Check if a string contains glob metacharacters (*, ?, [)
    pub fn contains_glob_metacharacters(pattern: &str) -> bool {
        pattern.contains('*') || pattern.contains('?') || pattern.contains('[')
    }

    /// Match a file name against a pattern.
    ///
    /// If the pattern contains glob metacharacters, uses glob matching.
    /// Otherwise uses substring matching for backward compatibility.
    /// Patterns without path separators match against just the filename portion.
    pub fn matches_pattern(file_name: &str, pattern: &str) -> bool {
        if contains_glob_metacharacters(pattern) {
            // Normalize both to forward slashes for glob matching
            let normalized_name = file_name.replace('\\', "/");
            let normalized_pattern = pattern.replace('\\', "/");

            // If pattern has no path separator, match against filename only
            let (name_to_match, pattern_to_use) = if !normalized_pattern.contains('/') {
                let filename = normalized_name
                    .rsplit('/')
                    .next()
                    .unwrap_or(&normalized_name);
                (filename.to_string(), normalized_pattern)
            } else {
                (normalized_name, normalized_pattern)
            };

            match glob::Pattern::new(&pattern_to_use) {
                Ok(glob_pattern) => glob_pattern.matches(&name_to_match),
                // Invalid glob pattern: fall back to substring matching
                Err(_) => file_name.contains(pattern),
            }
        } else {
            file_name.contains(pattern)
        }
    }

    /// Normalize a glob pattern for the `glob` crate (needs forward slashes).
    /// Preserves escaped backslashes (\\) used as glob escapes.
    fn normalize_glob_pattern(pattern: &str) -> String {
        pattern
            .replace("\\\\", "\x00") // Temporarily protect escaped backslashes
            .replace('\\', "/")
            .replace('\x00', "\\") // Restore escaped backslashes
    }

    /// Expand @response-file syntax, returning patterns as-is for archive matching.
    ///
    /// Does NOT expand glob patterns on the filesystem - used for
    /// list/extract/delete commands where patterns match archive entries.
    pub fn expand_response_files_for_archive(files: &[String]) -> Result<Vec<String>> {
        if files.len() == 1 && files[0].starts_with('@') {
            let response_file_path = &files[0][1..];
            let content = fs::read_to_string(response_file_path)
                .with_context(|| format!("Failed to read response file: {response_file_path}"))?;

            return Ok(content
                .lines()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(String::from)
                .collect());
        }

        if files.iter().any(|f| f.starts_with('@')) {
            bail!("Cannot mix @response-file with explicit file arguments");
        }

        Ok(files.to_vec())
    }

    /// Expand @response-file syntax and glob patterns for add operations.
    pub fn expand_response_files_with_stripping(files: &[String]) -> Result<Vec<PathBuf>> {
        if files.len() == 1 && files[0].starts_with('@') {
            return expand_response_file(&files[0][1..]);
        }

        if files.iter().any(|f| f.starts_with('@')) {
            bail!("Cannot mix @response-file with explicit file arguments");
        }

        expand_file_patterns(files)
    }

    fn expand_response_file(response_file_path: &str) -> Result<Vec<PathBuf>> {
        let content = fs::read_to_string(response_file_path)
            .with_context(|| format!("Failed to read response file: {response_file_path}"))?;

        let paths = content
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(PathBuf::from)
            .collect();

        Ok(paths)
    }

    fn expand_file_patterns(patterns: &[String]) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();

        for pattern in patterns {
            if contains_glob_metacharacters(pattern) {
                // Expand glob on the filesystem (e.g. "src/*.rs" -> list of files)
                paths.extend(expand_single_glob(pattern)?);
            } else {
                // Regular path - use as-is
                paths.push(PathBuf::from(pattern));
            }
        }

        Ok(paths)
    }

    fn expand_single_glob(pattern: &str) -> Result<Vec<PathBuf>> {
        let normalized_pattern = normalize_glob_pattern(pattern);
        let mut paths = Vec::new();

        let glob_iter = glob(&normalized_pattern)
            .with_context(|| format!("Invalid glob pattern: {pattern}"))?;

        for entry in glob_iter {
            match entry {
                Ok(path) => {
                    paths.push(path);
                }
                Err(e) => {
                    bail!("Error expanding glob pattern '{}': {}", pattern, e);
                }
            }
        }

        if paths.is_empty() {
            bail!("No files found matching pattern: {}", pattern);
        }

        Ok(paths)
    }

    /// Reject archive paths containing ".." (path traversal protection).
    ///
    /// A malicious archive could contain entries like "../../../etc/passwd"
    /// which would write outside the output directory during extraction.
    pub fn validate_archive_path(path: &str) -> Result<()> {
        let normalized = path.replace('\\', "/");
        for component in normalized.split('/') {
            if component == ".." {
                bail!(
                    "Path traversal detected in archive entry: {}",
                    normalize_path_for_display(path)
                );
            }
        }
        Ok(())
    }

    /// Convert path to backslashes for DAT archive storage
    pub fn normalize_path_for_archive(path: &str) -> String {
        path.replace('/', "\\")
    }

    /// Convert a DAT archive path (backslashes) to the OS path format
    pub fn to_system_path(dat_path: &str) -> PathBuf {
        PathBuf::from(dat_path.replace('\\', std::path::MAIN_SEPARATOR_STR))
    }

    /// Get just the filename (basename) from a path.
    /// Handles both forward and backward slashes.
    pub fn get_filename_from_dat_path(path: &str) -> &str {
        path.rfind(['/', '\\'])
            .map(|pos| &path[pos + 1..])
            .unwrap_or(path)
    }

    /// Get the directory part from a DAT archive path.
    /// Returns "." if the path has no directory component.
    pub fn get_dirname_from_dat_path(path: &str) -> &str {
        path.rfind(['/', '\\'])
            .map(|pos| &path[..pos])
            .unwrap_or(".")
    }

    /// Decode filename bytes from DAT files to ASCII strings.
    /// Strips C-style null terminators and rejects non-ASCII content.
    pub fn decode_filename(bytes: &[u8]) -> Result<String> {
        let trimmed_bytes: Vec<u8> = bytes.iter().take_while(|&&b| b != 0).copied().collect();

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

    /// Validate that a filename contains only ASCII characters.
    /// Used when reading from archives and when adding new files.
    pub fn validate_filename_ascii(filename: &str) -> Result<()> {
        if filename.is_ascii() {
            Ok(())
        } else {
            bail!("Non-ASCII filename found: {:?}", filename)
        }
    }

    /// Calculate the archive path for a file being added.
    ///
    /// Handles target directory placement and source path normalization.
    /// The result uses backslashes (DAT archive format).
    pub fn calculate_archive_path(
        file: &std::path::Path,
        base_path: &std::path::Path,
        target_dir: Option<&str>,
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
            None => strip_dot_prefix_from_path(&file.to_string_lossy()),
        };

        Ok(normalize_path_for_archive(&archive_path))
    }

    /// Normalize path separators to `/` and collapse consecutive slashes in a single pass
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
                }
                _ => {
                    result.push(ch);
                    last_was_slash = false;
                }
            }
        }

        result
    }

    /// Normalize a source path for archive storage.
    ///
    /// - "./patch000/file.txt" -> "patch000/file.txt"
    /// - ".\\patch000\\file.txt" -> "patch000/file.txt"
    /// - "/patch000/file.txt" -> "patch000/file.txt"
    /// - "C:\\patch000\\file.txt" -> "patch000/file.txt"
    /// - "patch000/file.txt" -> "patch000/file.txt" (no change)
    pub fn strip_dot_prefix_from_path(path: &str) -> String {
        let normalized = normalize_path_separators(path);
        let mut normalized_path = std::path::PathBuf::new();

        for component in std::path::Path::new(&normalized).components() {
            match component {
                std::path::Component::Prefix(_) => {}
                std::path::Component::RootDir => {}
                std::path::Component::CurDir if normalized_path.as_os_str().is_empty() => {}
                other => normalized_path.push(other.as_os_str()),
            }
        }

        normalize_path_separators(&normalized_path.to_string_lossy())
    }
}
