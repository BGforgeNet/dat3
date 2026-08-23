/*!
# Common Types and Utilities

Shared code for the DAT1, DAT2, and Arcanum formats. Provides a unified
`DatArchive` enum so callers don't need to know which format they're working with.
*/

use anyhow::{Context, Result, bail};
use glob::glob;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::arcanum::ArcanumArchive;
use crate::dat1::Dat1Archive;
use crate::dat2::Dat2Archive;

// DAT1 format detection: big-endian header, no signature to key on
const DAT1_MAX_DIRECTORIES: u32 = 1000;

/// Set once the reader has closed stdout, silencing every later write.
static STDOUT_CLOSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Write to stdout, going quiet on a broken pipe (e.g. when piped to `head`).
///
/// Every stdout write in the crate goes through this. Rust ignores SIGPIPE, so
/// an unguarded `println!` fails with `EPIPE` and panics, aborting the process.
///
/// Going quiet rather than exiting: for `x`, `a` and `d` stdout carries only
/// progress chatter while the real output is a file, so quitting on a closed
/// pipe would abandon a half-written archive - `a` piped to `head` would exit 0
/// having produced nothing. The pipe closing is a fact about the reporting
/// channel, not a reason to stop the work.
pub(crate) fn print_stdout(args: std::fmt::Arguments) {
    use std::sync::atomic::Ordering;

    if STDOUT_CLOSED.load(Ordering::Relaxed) {
        return;
    }
    if writeln!(io::stdout(), "{args}").is_err() {
        STDOUT_CLOSED.store(true, Ordering::Relaxed);
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
/// Used by all supported archive formats. For files already in an archive,
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

/// Controls how the `l` command renders its listing
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ListFormat {
    /// Aligned columns for reading
    Text,
    /// JSON array for another program to parse
    Json,
}

// ── DatArchive enum ────────────────────────────────────────────────

/// Unified interface for DAT1, DAT2, and Arcanum archives.
///
/// Uses an enum instead of trait objects because the set of known formats
/// is small and fixed - this gives us static dispatch, exhaustive matching,
/// and no heap allocation for the wrapper.
///
/// **Memory**: The entire archive is loaded into memory on open, whatever the
/// format. Fallout archives typically stay under ~200MB; retail Arcanum
/// archives run considerably larger and are held in RAM the same way.
///
/// ```ignore
/// let archive = DatArchive::open("master.dat")?;  // auto-detects format
/// let dat1 = DatArchive::new_dat1();               // create new DAT1
/// let dat2 = DatArchive::new_dat2();               // create new DAT2
/// let arc = DatArchive::new_arcanum();             // create new Arcanum
/// ```
pub enum DatArchive {
    /// Fallout 1 format (big-endian, hierarchical dirs, LZSS compression)
    Dat1(Dat1Archive),
    /// Fallout 2 format (little-endian, flat file list, zlib compression)
    Dat2(Dat2Archive),
    /// Arcanum format (little-endian, flat entry table, zlib compression)
    Arcanum(ArcanumArchive),
}

impl DatArchive {
    /// Open an existing DAT archive, auto-detecting the format
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = fs::read(&path)
            .with_context(|| format!("Failed to read DAT file: {}", path.as_ref().display()))?;

        // Arcanum is the only format with a real magic, so its check is
        // authoritative and goes first; DAT1 is a header heuristic and DAT2
        // (which has no signature at all) is the fallback.
        if crate::arcanum::is_arcanum_format(&data) {
            Ok(Self::Arcanum(ArcanumArchive::from_bytes(data)?))
        } else if Self::is_dat1_format(&data) {
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

    /// Create a new empty Arcanum archive
    pub fn new_arcanum() -> Self {
        Self::Arcanum(ArcanumArchive::new())
    }

    /// Check if this is a DAT1 archive
    pub fn is_dat1(&self) -> bool {
        matches!(self, Self::Dat1(_))
    }

    /// Human-readable format name for error messages
    pub fn format_name(&self) -> &'static str {
        match self {
            Self::Dat1(_) => "DAT1",
            Self::Dat2(_) => "DAT2",
            Self::Arcanum(_) => "Arcanum",
        }
    }

    /// Detect DAT1 format by examining the big-endian header.
    ///
    /// The header opens with a directory count and the engine's allocation hint
    /// for that directory list, which is never below the count. Both are checked:
    /// DAT2 carries no signature at all and is the fallback, so this heuristic is
    /// what keeps a DAT2 archive from being parsed as DAT1.
    ///
    /// The second field is NOT a format identifier, despite reading like one in
    /// the shipped archives - `critter.dat` carries 10 and `master.dat` 94, which
    /// are simply their own hints. Matching those two values exactly rejects every
    /// other real DAT1 archive, including the Fallout 1 demo's (hint 46).
    fn is_dat1_format(data: &[u8]) -> bool {
        let Some(header) = data.get(..16) else {
            return false;
        };
        let field =
            |i: usize| u32::from_be_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
        let dir_count = field(0);
        let allocation_hint = field(4);

        dir_count > 0
            && dir_count < DAT1_MAX_DIRECTORIES
            && allocation_hint >= dir_count
            && allocation_hint < DAT1_MAX_DIRECTORIES
    }

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, files: &[String], format: ListFormat) -> Result<()> {
        match self {
            Self::Dat1(a) => a.list(files, format),
            Self::Dat2(a) => a.list(files, format),
            Self::Arcanum(a) => a.list(files, format),
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
            Self::Arcanum(a) => a.extract(output_dir.as_ref(), files, mode),
        }
    }

    /// Add a file to the archive (directories are processed recursively)
    pub fn add_file<P: AsRef<Path>>(
        &mut self,
        file_path: P,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        source_root: Option<&Path>,
    ) -> Result<()> {
        match self {
            Self::Dat1(a) => a.add_file(file_path.as_ref(), compression, target_dir, source_root),
            Self::Dat2(a) => a.add_file(file_path.as_ref(), compression, target_dir, source_root),
            Self::Arcanum(a) => {
                a.add_file(file_path.as_ref(), compression, target_dir, source_root)
            }
        }
    }

    /// Delete a file from the archive
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        match self {
            Self::Dat1(a) => a.delete_file(file_name),
            Self::Dat2(a) => a.delete_file(file_name),
            Self::Arcanum(a) => a.delete_file(file_name),
        }
    }

    /// Save the archive to a file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        match self {
            Self::Dat1(a) => a.save(path.as_ref()),
            Self::Dat2(a) => a.save(path.as_ref()),
            Self::Arcanum(a) => a.save(path.as_ref()),
        }
    }
}

// ── Shared archive operations ──────────────────────────────────────

/// List files using shared filter-and-print logic.
///
/// All formats use this same flow:
/// normalize patterns -> filter entries -> print listing -> report missing.
pub fn list_files_filtered(
    all_files: &[&FileEntry],
    patterns: &[String],
    format: ListFormat,
) -> Result<()> {
    let normalized_patterns = utils::normalize_user_patterns(patterns);

    let (files_to_list, missing_patterns) =
        filter_and_track_patterns(all_files, &normalized_patterns, |file, pattern| {
            utils::matches_pattern(&file.name, pattern)
        });

    match format {
        ListFormat::Text => utils::print_file_listing(&files_to_list),
        ListFormat::Json => utils::print_file_listing_json(&files_to_list),
    }

    report_missing_patterns(&missing_patterns)
}

/// Report patterns that matched no entry and fail.
///
/// Shared by the list and extract paths so both reject a mistyped name the
/// same way.
fn report_missing_patterns(missing_patterns: &[String]) -> Result<()> {
    if missing_patterns.is_empty() {
        return Ok(());
    }

    eprintln!("\nFiles not found:");
    for pattern in missing_patterns {
        let display = utils::normalize_path_for_display(pattern);
        eprintln!("  {display}");
    }
    bail!("Some requested files were not found");
}

/// Filter files by patterns and return matched files, failing if any pattern
/// matched nothing.
///
/// Shared by all formats' extract paths. Checked before extraction starts, so a
/// mistyped name leaves no half-populated output directory.
/// Accepts owned entries or borrowed ones, so a format holding its files in
/// per-directory lists can filter without first cloning them into a flat `Vec`.
pub fn filter_files_by_patterns<'a, T: AsRef<FileEntry>>(
    all_files: &'a [T],
    patterns: &[String],
) -> Result<Vec<&'a FileEntry>> {
    let normalized_patterns = utils::normalize_user_patterns(patterns);

    let (filtered, missing_patterns) =
        filter_and_track_patterns(all_files, &normalized_patterns, |file, pattern| {
            utils::matches_pattern(&file.as_ref().name, pattern)
        });

    report_missing_patterns(&missing_patterns)?;

    Ok(filtered.into_iter().map(|file| file.as_ref()).collect())
}

/// Extract entries in parallel, decompressing each compressed one with
/// `decompress`.
///
/// Shared by all three formats. They differ only in that codec: the entry table
/// is already parsed into `FileEntry` by this point, and every format resolves
/// its payload bytes through `utils::read_file_slice`.
pub fn extract_archive_parallel(
    archive_data: &[u8],
    files_to_extract: &[&FileEntry],
    output_dir: &Path,
    mode: ExtractionMode,
    decompress: impl Fn(&[u8], usize) -> Result<Vec<u8>> + Sync,
) -> Result<()> {
    use rayon::prelude::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let total_files = files_to_extract.len();
    let completed = AtomicUsize::new(0);

    print_stdout(format_args!("Extracting {total_files} files..."));
    let start = std::time::Instant::now();

    files_to_extract
        .par_iter()
        .try_for_each(|file| -> Result<()> {
            utils::validate_archive_path(&file.name)?;

            // Progress reporting every 1000 files
            let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if count.is_multiple_of(1000) || count == total_files {
                let elapsed = start.elapsed().as_millis();
                let files_per_sec = count as f64 / elapsed as f64 * 1000.0;
                print_stdout(format_args!(
                    "Progress: {count}/{total_files} files extracted ({files_per_sec:.1} files/sec)"
                ));
            }

            let output_path = utils::resolve_output_path(output_dir, &file.name, mode);

            utils::ensure_dir_exists(&output_path)?;

            // Read and optionally decompress
            let file_data = utils::read_file_slice(archive_data, file)
                .with_context(|| format!("Failed to read data for file '{}'", file.name))?;
            let final_data = if file.compressed {
                decompress(&file_data, file.size as usize)
                    .with_context(|| format!("Failed to decompress {}", file.name))?
            } else {
                file_data
            };

            fs::write(&output_path, final_data)
                .with_context(|| format!("Failed to write {}", output_path.display()))?;

            Ok(())
        })?;

    let total_time = start.elapsed();
    print_stdout(format_args!(
        "Extraction completed in {:.2}s",
        total_time.as_secs_f64()
    ));
    Ok(())
}

/// Read files from disk into an entry list: zlib-compress when it saves
/// space, replace same-named entries, dedupe the batch, and keep the list
/// sorted case-insensitively as the zlib-based formats require.
///
/// Shared by the DAT2 and Arcanum add paths.
pub fn add_files_zlib(
    entries: &mut Vec<FileEntry>,
    file_path: &Path,
    compression: CompressionLevel,
    target_dir: Option<&str>,
    source_root: Option<&Path>,
) -> Result<()> {
    use rayon::prelude::*;
    use std::collections::HashSet;

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
            process_single_file_for_adding(file, base_path, compression, target_dir, source_root)
        })
        .collect();

    let new_entries = results?; // Collect results, propagating the first error if any file failed

    // Remove existing files that match new file names
    let new_file_names: HashSet<String> = new_entries.iter().map(|e| e.name.clone()).collect();
    entries.retain(|existing_file| !new_file_names.contains(&existing_file.name));

    // Add new files, deduplicating within the batch (keep first occurrence).
    // This can happen if the user passes the same file or two files with the same name.
    let mut seen_names = HashSet::new();
    for entry in new_entries {
        if seen_names.insert(entry.name.clone()) {
            entries.push(entry);
        }
    }

    // The formats require entries sorted alphabetically (case-insensitive)
    entries.sort_by_key(|f| f.name.to_lowercase());

    Ok(())
}

/// Process a single file for adding to an archive
fn process_single_file_for_adding(
    file: &Path,
    base_path: &Path,
    compression: CompressionLevel,
    target_dir: Option<&str>,
    source_root: Option<&Path>,
) -> Result<FileEntry> {
    let data = fs::read(file).with_context(|| format!("Failed to read {}", file.display()))?;
    let archive_path = utils::calculate_archive_path(file, base_path, target_dir, source_root)?;
    let display_path = utils::normalize_path_for_display(&archive_path);
    print_stdout(format_args!("Adding: {display_path}"));

    if compression.level() > 0 {
        let compressed_data = compress_zlib(&data, compression.level())?;
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

/// Compress data using zlib
fn compress_zlib(data: &[u8], level: u8) -> Result<Vec<u8>> {
    let mut encoder =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(level as u32));
    encoder.write_all(data)?;
    encoder.finish().context("Failed to compress with zlib")
}

/// Decompress zlib data with a pre-allocated output buffer
pub fn decompress_zlib(data: &[u8], expected_size: usize) -> Result<Vec<u8>> {
    use std::io::Read;

    let mut decoder = flate2::read::ZlibDecoder::new(data);
    // expected_size is untrusted archive metadata, so cap the reservation by
    // deflate's maximum expansion of ~1032:1 (raw deflate stores 8 bits per
    // symbol at minimum overhead).
    let mut decompressed = Vec::with_capacity(expected_size.min(data.len().saturating_mul(1032)));
    decoder
        .read_to_end(&mut decompressed)
        .context("Failed to decompress zlib data")?;
    Ok(decompressed)
}

/// Delete a file from a list by normalized name.
///
/// Shared by DAT1 and DAT2 delete implementations.
pub fn delete_file_from_list(files: &mut Vec<FileEntry>, file_name: &str) -> Result<()> {
    let normalized_name = utils::normalize_user_path(file_name).into_owned();

    if let Some(pos) = files.iter().position(|f| f.name == normalized_name) {
        let display_name = utils::normalize_path_for_display(&normalized_name);
        print_stdout(format_args!("Deleting: {display_name}"));
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

    /// Append `value` to `out` as a quoted JSON string.
    ///
    /// Archive names come from a third-party file, so a name holding a quote,
    /// backslash, or control byte would otherwise emit a document the consumer
    /// cannot parse. Non-ASCII is passed through: JSON is UTF-8 and `name` is
    /// already a `String`.
    fn push_json_string(out: &mut String, value: &str) {
        out.push('"');
        for c in value.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
    }

    /// Render the listing as a JSON array, one entry object per line.
    ///
    /// Separators are always forward slashes, unlike the text listing, which
    /// keeps the platform's own: this output is data for another program, so
    /// the same archive has to describe itself identically everywhere. The
    /// names it emits are what the `x`, `e`, and `d` commands accept back.
    pub fn format_file_listing_json<T: AsRef<FileEntry>>(files: &[T]) -> String {
        if files.is_empty() {
            return "[]".to_string();
        }

        let mut out = String::from("[\n");
        for (i, file) in files.iter().enumerate() {
            let file = file.as_ref();
            out.push_str("  {\"name\": ");
            push_json_string(&mut out, &file.name.replace('\\', "/"));
            out.push_str(&format!(
                ", \"size\": {}, \"packed_size\": {}, \"compressed\": {}}}",
                file.size, file.packed_size, file.compressed
            ));
            if i + 1 != files.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push(']');
        out
    }

    /// Print the JSON listing
    pub fn print_file_listing_json<T: AsRef<FileEntry>>(files: &[T]) {
        print_stdout(format_args!("{}", format_file_listing_json(files)));
    }

    /// Stream archive bytes to a same-directory temp file via the given closure,
    /// then rename it over the target, so an interrupted save cannot destroy an
    /// existing archive. Streaming keeps peak memory at one file's data instead
    /// of buffering the whole archive.
    pub fn write_atomically(
        path: &Path,
        write: impl FnOnce(&mut std::io::BufWriter<fs::File>) -> Result<()>,
    ) -> Result<()> {
        let file_name = path
            .file_name()
            .with_context(|| format!("Invalid archive path: {}", path.display()))?;
        let mut tmp_name = std::ffi::OsString::from(".");
        tmp_name.push(file_name);
        tmp_name.push(".tmp");
        let tmp_path = path.with_file_name(tmp_name);

        let result = fs::File::create(&tmp_path)
            .with_context(|| format!("Failed to create {}", tmp_path.display()))
            .and_then(|file| {
                let mut writer = std::io::BufWriter::new(file);
                write(&mut writer)?;
                writer
                    .flush()
                    .with_context(|| format!("Failed to write {}", tmp_path.display()))
            });
        if let Err(e) = result {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }

        if let Err(e) = fs::rename(&tmp_path, path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(e)
                .with_context(|| format!("Failed to move archive into place: {}", path.display()));
        }
        Ok(())
    }

    /// Resolve the on-disk output path for an archive entry being extracted.
    pub fn resolve_output_path(
        output_dir: &Path,
        archive_name: &str,
        mode: ExtractionMode,
    ) -> PathBuf {
        match mode {
            ExtractionMode::Flat => output_dir.join(get_filename_from_dat_path(archive_name)),
            ExtractionMode::PreserveStructure => output_dir.join(to_system_path(archive_name)),
        }
    }

    /// Read one entry's raw (possibly still compressed) bytes: in-memory data
    /// for newly added entries, a bounds-checked slice of the archive buffer otherwise.
    pub fn read_file_slice(archive_data: &[u8], file: &FileEntry) -> Result<Vec<u8>> {
        if let Some(ref data) = file.data {
            return Ok(data.clone());
        }

        // Checked math: offset/packed_size come from the archive file and can be
        // hostile; try_from also guards 32-bit targets where u64 -> usize narrows.
        let out_of_bounds = || {
            anyhow::anyhow!(
                "File data extends beyond archive: {} (offset: {}, size: {})",
                file.name,
                file.offset,
                file.packed_size
            )
        };
        let start = usize::try_from(file.offset).map_err(|_| out_of_bounds())?;
        let end = start
            .checked_add(file.packed_size as usize)
            .ok_or_else(out_of_bounds)?;

        if end > archive_data.len() {
            return Err(out_of_bounds());
        }

        Ok(archive_data[start..end].to_vec())
    }

    /// Collect all files from a path (file or directory, recursive).
    /// Validates that all filenames are ASCII-only.
    pub fn collect_files<P: AsRef<Path>>(path: P) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        collect_files_inner(path.as_ref(), &mut files)?;
        Ok(files)
    }

    /// Inner recursive worker for `collect_files`.
    ///
    /// Validates ASCII at the leaf push site so each path is checked exactly once.
    fn collect_files_inner(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                bail!("Path does not exist: {}", path.display());
            }
            Err(e) => {
                return Err(e)
                    .with_context(|| format!("Failed to inspect path: {}", path.display()));
            }
        };

        if metadata.file_type().is_symlink() {
            // Distinguish dangling symlinks (target missing) from non-dangling ones.
            match path.try_exists() {
                Ok(true) => eprintln!("Skipping symlink: {}", path.display()),
                _ => eprintln!("Skipping dangling symlink: {}", path.display()),
            }
            return Ok(());
        }

        if metadata.is_file() {
            let path_str = path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid path encoding: {}", path.display()))?;
            validate_filename_ascii(path_str)
                .with_context(|| format!("Invalid path: {}", path.display()))?;
            out.push(path.to_path_buf());
        } else if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let entry_path = entry.path();
                let entry_metadata = fs::symlink_metadata(&entry_path)
                    .with_context(|| format!("Failed to inspect path: {}", entry_path.display()))?;

                if entry_metadata.file_type().is_symlink() {
                    match entry_path.try_exists() {
                        Ok(true) => eprintln!("Skipping symlink: {}", entry_path.display()),
                        _ => eprintln!("Skipping dangling symlink: {}", entry_path.display()),
                    }
                } else if entry_metadata.is_file() {
                    let path_str = entry_path.to_str().ok_or_else(|| {
                        anyhow::anyhow!("Invalid path encoding: {}", entry_path.display())
                    })?;
                    validate_filename_ascii(path_str)
                        .with_context(|| format!("Invalid path: {}", entry_path.display()))?;
                    out.push(entry_path);
                } else if entry_metadata.is_dir() {
                    collect_files_inner(&entry_path, out)?;
                }
            }
        }

        Ok(())
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
    pub fn expand_response_files_with_stripping(
        files: &[String],
        change_dir: Option<&Path>,
    ) -> Result<Vec<PathBuf>> {
        if files.len() == 1 && files[0].starts_with('@') {
            return expand_response_file(&files[0][1..], change_dir);
        }

        if files.iter().any(|f| f.starts_with('@')) {
            bail!("Cannot mix @response-file with explicit file arguments");
        }

        expand_file_patterns(files, change_dir)
    }

    fn expand_response_file(
        response_file_path: &str,
        change_dir: Option<&Path>,
    ) -> Result<Vec<PathBuf>> {
        let content = fs::read_to_string(response_file_path)
            .with_context(|| format!("Failed to read response file: {response_file_path}"))?;

        let paths: Vec<String> = content
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(String::from)
            .collect();

        expand_file_patterns(&paths, change_dir)
    }

    /// Expand glob patterns and join relative patterns against `-C`.
    ///
    /// For relative patterns under `-C`, rejects components other than `Normal`
    /// (e.g. `..`) up-front so glob expansion cannot silently walk outside the
    /// `-C` directory. Does NOT canonicalize, check symlinks, or bounds-check
    /// the final resolved paths — callers must pipe every result through
    /// `resolve_add_input_path` for the full security gate.
    fn expand_file_patterns(
        patterns: &[String],
        change_dir: Option<&Path>,
    ) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();

        for pattern in patterns {
            let resolved_pattern = if let Some(base_dir) = change_dir {
                let pattern_path = Path::new(pattern);
                if pattern_path.is_absolute() {
                    // Pass absolute paths through unchanged; security validation
                    // is deferred to resolve_add_input_path.
                    pattern_path.to_path_buf()
                } else {
                    validate_change_dir_operand(pattern_path)?;
                    base_dir.join(pattern)
                }
            } else {
                PathBuf::from(pattern)
            };

            if contains_glob_metacharacters(pattern) {
                // Expand glob on the filesystem (e.g. "src/*.rs" -> list of files)
                paths.extend(expand_single_glob(&resolved_pattern)?);
            } else {
                // Regular path - use as-is
                paths.push(resolved_pattern);
            }
        }

        Ok(paths)
    }

    fn expand_single_glob(pattern: &Path) -> Result<Vec<PathBuf>> {
        let display_pattern = pattern.display().to_string();
        let normalized_pattern = normalize_glob_pattern(&display_pattern);
        let mut paths = Vec::new();

        let glob_iter = glob(&normalized_pattern)
            .with_context(|| format!("Invalid glob pattern: {display_pattern}"))?;

        for entry in glob_iter {
            match entry {
                Ok(path) => {
                    paths.push(path);
                }
                Err(e) => {
                    bail!("Error expanding glob pattern '{}': {}", display_pattern, e);
                }
            }
        }

        if paths.is_empty() {
            bail!("No files found matching pattern: {}", display_pattern);
        }

        Ok(paths)
    }

    /// True for a Windows drive prefix such as `C:`.
    ///
    /// Checked by hand because `Component::Prefix` is produced only by the
    /// Windows implementation of `Path`: on a Unix build `C:\x.txt` parses as
    /// one ordinary component, so the same archive that stays inside the output
    /// directory here escapes it on Windows. An archive is portable, so the
    /// shape is rejected on every host.
    fn is_drive_prefix(component: &str) -> bool {
        let bytes = component.as_bytes();
        bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
    }

    /// Split an archive path into its safe components, rejecting every shape
    /// that would let it escape the directory it is resolved against: `..`,
    /// an absolute root, and a drive prefix. `.` components are dropped.
    ///
    /// Shared by both directions - entries read out of an archive and paths
    /// being written into one - so the two cannot drift apart again. The extract
    /// side is the security-critical caller: its input is attacker-supplied, and
    /// `Path::join` silently replaces the base when handed an absolute path.
    fn archive_path_parts(path: &str) -> Result<Vec<String>> {
        let normalized = normalize_path_separators(path);
        let mut parts: Vec<String> = Vec::new();

        for component in Path::new(&normalized).components() {
            match component {
                std::path::Component::Normal(s) => {
                    let part = s.to_str().unwrap_or_default();
                    if is_drive_prefix(part) {
                        bail!("drive prefix '{part}'");
                    }
                    parts.push(part.to_string());
                }
                std::path::Component::CurDir => {
                    // silently skip '.' components
                }
                std::path::Component::ParentDir => bail!("'..' component"),
                std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                    bail!("absolute path")
                }
            }
        }

        Ok(parts)
    }

    /// Reject an archive entry name that would extract outside the output
    /// directory - `..`, an absolute root, or a drive prefix.
    ///
    /// A malicious archive could store an entry as `../../../etc/passwd` or as
    /// `\tmp\x.txt`; the second is the more dangerous shape, because `Path::join`
    /// discards the output directory rather than nesting under it.
    pub fn validate_archive_path(path: &str) -> Result<()> {
        archive_path_parts(path).map_err(|e| {
            anyhow::anyhow!(
                "Path traversal detected in archive entry ({e}): {}",
                normalize_path_for_display(path)
            )
        })?;
        Ok(())
    }

    /// Validate and normalize a path to be stored in a new archive.
    ///
    /// - Rejects `..` (ParentDir), absolute roots, and Windows drive prefixes.
    /// - `.` (CurDir) components are silently removed.
    /// - Returns the normalized path string with components joined by `/`.
    /// - Returns an error if the post-normalization result is empty.
    pub fn validate_add_archive_path(path: &str) -> Result<String> {
        let normalized = normalize_path_separators(path);
        if normalized.is_empty() {
            bail!("Invalid archive path: path is empty");
        }

        let parts = archive_path_parts(path).map_err(|e| {
            anyhow::anyhow!(
                "Invalid archive path for add operation ({e}): {}",
                normalize_path_for_display(path)
            )
        })?;

        let result = parts.join("/");
        if result.is_empty() {
            bail!("Invalid archive path: path is empty after normalization");
        }

        Ok(result)
    }

    /// Resolve an add operand against `-C`, rejecting operands that escape it.
    ///
    /// This is the sole security gate for add operands under `-C`. It:
    /// - canonicalizes the path to resolve any `..` components,
    /// - rejects symlinks (dangling or not) to prevent link-following attacks,
    /// - rejects paths whose canonical form falls outside `change_dir`.
    ///
    /// `expand_file_patterns` intentionally skips these checks and delegates
    /// them here so that validation happens exactly once per resolved path.
    pub fn resolve_add_input_path(path: &Path, change_dir: Option<&Path>) -> Result<PathBuf> {
        let Some(base_dir) = change_dir else {
            return Ok(path.to_path_buf());
        };

        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            base_dir.join(path)
        };

        // Check if the path is a symlink before resolving - symlinks are always skipped
        let metadata = fs::symlink_metadata(&candidate)
            .with_context(|| format!("Failed to inspect path: {}", candidate.display()))?;

        if metadata.file_type().is_symlink() {
            bail!(
                "Symlinks are not allowed in add operations: {}",
                normalize_path_for_display(&path.display().to_string())
            );
        }

        // Canonicalize both paths for proper comparison
        let resolved = fs::canonicalize(&candidate)
            .with_context(|| format!("Failed to resolve add path: {}", candidate.display()))?;

        let canonical_base = fs::canonicalize(base_dir).with_context(|| {
            format!(
                "Failed to canonicalize base directory: {}",
                base_dir.display()
            )
        })?;

        if !resolved.starts_with(&canonical_base) {
            bail!(
                "Add path escapes -C directory: {}",
                normalize_path_for_display(&path.display().to_string())
            );
        }

        Ok(resolved)
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
        source_root: Option<&std::path::Path>,
    ) -> Result<String> {
        let archive_path = match source_root {
            Some(root) => {
                let relative_path = file.strip_prefix(root).with_context(|| {
                    format!(
                        "Resolved path '{}' is outside source root '{}'",
                        file.display(),
                        root.display()
                    )
                })?;
                let relative_path = normalize_path_separators(&relative_path.to_string_lossy());

                match target_dir {
                    Some(target) => format!("{target}/{relative_path}"),
                    None => relative_path,
                }
            }
            None => match target_dir {
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
                            .ok_or_else(|| {
                                anyhow::anyhow!("Invalid filename for: {}", file.display())
                            })?
                            .to_string_lossy();
                        format!("{target}/{filename}")
                    }
                }
                None => strip_dot_prefix_from_path(&file.to_string_lossy()),
            },
        };

        let archive_path = validate_add_archive_path(&archive_path)?;
        Ok(normalize_path_for_archive(&archive_path))
    }

    fn validate_change_dir_operand(path: &Path) -> Result<()> {
        if path.as_os_str().is_empty() {
            bail!("Empty add path is not allowed with -C");
        }

        for component in path.components() {
            match component {
                std::path::Component::Normal(_) => {}
                _ => {
                    bail!(
                        "Invalid add path with -C: {}",
                        normalize_path_for_display(&path.display().to_string())
                    );
                }
            }
        }

        Ok(())
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
