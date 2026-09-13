/*!
# Common Types and Utilities

Shared code for the DAT1, DAT2, Arcanum, and ToEE formats: entry types, the
extract/add/delete engines, codecs, and path utilities. The unified archive
interface lives in `archive`.
*/

use anyhow::{Context, Result, bail};
use glob::glob;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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

/// Longest entry path any format reads or writes: a backstop on parser memory
/// rather than a format limit. Names are length-prefixed by untrusted archive
/// metadata, and unbounded ToEE depth once turned a 2.9 MB archive into 14.3 GB
/// of path strings; shipped archives peak at 111 bytes.
pub const MAX_PATH_BYTES: usize = 1024;

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
    ///
    /// Sizes are narrowed to u32 here: `utils::read_entry_data` refuses larger
    /// inputs, and compressed data is only kept when it is smaller than its input.
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

/// What to do about a requested name or glob that matches nothing in the archive
#[derive(Debug, Clone, Copy)]
pub enum MissingFiles {
    /// Report the misses and fail without listing or extracting anything
    Fail,
    /// Report the misses as a warning and carry on with whatever did match
    Warn,
}

/// Controls how the `l` command renders its listing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormat {
    /// Aligned columns for reading
    Text,
    /// JSON array for another program to parse
    Json,
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
    on_missing: MissingFiles,
) -> Result<()> {
    let compiled = utils::compile_patterns(patterns)?;

    let (files_to_list, missing_patterns) =
        filter_and_track_patterns(all_files, &compiled, |file, pattern| {
            pattern.matches(&file.name)
        });

    match format {
        ListFormat::Text => utils::print_file_listing(&files_to_list),
        ListFormat::Json => print_stdout(format_args!(
            "{}",
            utils::format_file_listing_json(&files_to_list)
        )),
    }

    report_missing_patterns(&missing_patterns, on_missing)
}

/// Report patterns that matched no entry, failing unless the caller tolerates
/// misses.
///
/// Shared by the list and extract paths so both treat a mistyped name the same
/// way. The names are printed either way; only the exit status differs.
fn report_missing_patterns(
    missing_patterns: &[&utils::NamePattern],
    on_missing: MissingFiles,
) -> Result<()> {
    if missing_patterns.is_empty() {
        return Ok(());
    }

    match on_missing {
        MissingFiles::Fail => eprintln!("\nFiles not found:"),
        MissingFiles::Warn => eprintln!("\nWarning: files not found:"),
    }
    for pattern in missing_patterns {
        let display = utils::normalize_path_for_display(pattern.source());
        eprintln!("  {display}");
    }

    match on_missing {
        MissingFiles::Fail => bail!("Some requested files were not found"),
        MissingFiles::Warn => Ok(()),
    }
}

/// Filter files by patterns and return matched files, failing if any pattern
/// matched nothing and `on_missing` is `Fail`.
///
/// Shared by all formats' extract paths. Checked before extraction starts, so a
/// mistyped name leaves no half-populated output directory.
/// Accepts owned entries or borrowed ones, so a format holding its files in
/// per-directory lists can filter without first cloning them into a flat `Vec`.
pub fn filter_files_by_patterns<'a, T: AsRef<FileEntry>>(
    all_files: &'a [T],
    patterns: &[String],
    on_missing: MissingFiles,
) -> Result<Vec<&'a FileEntry>> {
    let compiled = utils::compile_patterns(patterns)?;

    let (filtered, missing_patterns) =
        filter_and_track_patterns(all_files, &compiled, |file, pattern| {
            pattern.matches(&file.as_ref().name)
        });

    report_missing_patterns(&missing_patterns, on_missing)?;

    Ok(filtered.into_iter().map(|file| file.as_ref()).collect())
}

/// Extraction progress, with a rate only once any time has measurably passed
pub fn progress_line(count: usize, total: usize, elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs_f64();
    if seconds > 0.0 {
        let rate = count as f64 / seconds;
        format!("Progress: {count}/{total} files extracted ({rate:.1} files/sec)")
    } else {
        format!("Progress: {count}/{total} files extracted")
    }
}

/// Warn that flat extraction skips entries whose file name a later entry reuses
fn report_flat_name_collisions(replaced: &[&FileEntry]) {
    const SHOWN: usize = 5;
    if replaced.is_empty() {
        return;
    }
    eprintln!(
        "Warning: {} entries share a file name with a later entry and are not extracted in flat mode:",
        replaced.len()
    );
    for file in replaced.iter().take(SHOWN) {
        eprintln!("  {}", utils::normalize_path_for_display(&file.name));
    }
    if replaced.len() > SHOWN {
        eprintln!("  ...and {} more", replaced.len() - SHOWN);
    }
}

/// Extract entries in parallel, decompressing each compressed one with
/// `decompress`.
///
/// Shared by all four formats. They differ only in that codec: the entry table
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

    let kept;
    let files_to_extract = match mode {
        ExtractionMode::PreserveStructure => files_to_extract,
        ExtractionMode::Flat => {
            let replaced;
            (kept, replaced) = utils::last_entry_per_flat_name(files_to_extract);
            report_flat_name_collisions(&replaced);
            &kept
        }
    };

    let total_files = files_to_extract.len();
    let completed = AtomicUsize::new(0);

    print_stdout(format_args!("Extracting {total_files} files..."));
    let start = std::time::Instant::now();

    files_to_extract
        .par_iter()
        .try_for_each(|file| -> Result<()> {
            utils::validate_archive_path(&file.name)?;

            let output_path = utils::resolve_output_path(output_dir, &file.name, mode);

            utils::ensure_dir_exists(&output_path)?;

            // Read and optionally decompress
            let file_data = utils::read_file_slice(archive_data, file)
                .with_context(|| format!("Failed to read data for file '{}'", file.name))?;
            let write_result = if file.compressed {
                let decompressed = decompress(file_data, file.size as usize)
                    .with_context(|| format!("Failed to decompress {}", file.name))?;
                fs::write(&output_path, decompressed)
            } else {
                fs::write(&output_path, file_data)
            };
            write_result.with_context(|| format!("Failed to write {}", output_path.display()))?;

            // Counted once written, every 1000 files and at the end
            let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if count.is_multiple_of(1000) || count == total_files {
                print_stdout(format_args!(
                    "{}",
                    progress_line(count, total_files, start.elapsed())
                ));
            }

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
/// Shared by the DAT2, Arcanum, and ToEE add paths.
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

    // The formats require entries sorted alphabetically (case-insensitive).
    // Cached: the key allocates, and sort_by_key recomputes it per comparison
    // rather than per element.
    entries.sort_by_cached_key(|f| f.name.to_lowercase());

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
    let data = utils::read_entry_data(file)?;
    let archive_path = utils::calculate_archive_path(file, base_path, target_dir, source_root)?;
    let display_path = utils::normalize_path_for_display(&archive_path);
    // The readers reject longer names, so writing one would produce an archive
    // dat3 itself cannot open.
    if archive_path.len() > MAX_PATH_BYTES {
        bail!("Archive path is longer than {MAX_PATH_BYTES} bytes: {display_path}");
    }
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

/// Word a parse error from a deku-derived record for the user. deku reports a
/// short read in bits, which says nothing useful about a truncated archive.
pub fn deku_parse_error(context: impl std::fmt::Display, e: deku::DekuError) -> anyhow::Error {
    // DekuError is non_exhaustive, so a catch-all arm is required; it keeps deku's own wording.
    #[expect(clippy::wildcard_enum_match_arm)]
    match e {
        deku::DekuError::Incomplete(need) => anyhow::anyhow!(
            "{context}: archive data ends early ({} more bytes needed)",
            need.byte_size()
        ),
        deku::DekuError::Assertion(check) => anyhow::anyhow!("{context}: invalid value ({check})"),
        other => anyhow::anyhow!("{context}: {other}"),
    }
}

/// Check decompressed output against the size its entry declares.
///
/// The declared size is untrusted, so decoders stop once output passes it
/// rather than expanding a crafted stream in full. Real archives match exactly -
/// every compressed entry in the archives the integration suite extracts does -
/// so a mismatch marks a corrupt entry.
pub fn check_decompressed_len(len: usize, expected_size: usize) -> Result<()> {
    if len > expected_size {
        bail!("Decompressed data exceeds the declared size of {expected_size} bytes");
    }
    if len < expected_size {
        bail!("Decompressed data is {len} bytes, but the archive declares {expected_size}");
    }
    Ok(())
}

/// Decompress zlib data, which must expand to exactly `expected_size` bytes
pub fn decompress_zlib(data: &[u8], expected_size: usize) -> Result<Vec<u8>> {
    use std::io::Read;

    // Reading one byte past the declared size is enough to detect an overrun
    // without decoding the rest of the stream.
    let limit = (expected_size as u64).saturating_add(1);
    let mut decoder = flate2::read::ZlibDecoder::new(data).take(limit);
    // expected_size is untrusted archive metadata, so cap the reservation by
    // deflate's maximum expansion of ~1032:1 (raw deflate stores 8 bits per
    // symbol at minimum overhead).
    let mut decompressed = Vec::with_capacity(expected_size.min(data.len().saturating_mul(1032)));
    decoder
        .read_to_end(&mut decompressed)
        .context("Failed to decompress zlib data")?;
    check_decompressed_len(decompressed.len(), expected_size)?;
    Ok(decompressed)
}

/// Delete a file from a list by normalized name.
///
/// Shared by the DAT2, Arcanum, and ToEE delete implementations; DAT1 keeps its
/// files per directory and deletes through its own.
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

/// Resolve `d` operands to the names of the entries they delete.
///
/// A glob selects every entry it matches. A plain name selects only the entry
/// with exactly that name: the substring matching `l` and `x` apply would delete
/// unrelated files. Any operand that selects nothing fails the whole command.
pub fn resolve_delete_targets(names: &[&str], patterns: &[String]) -> Result<Vec<String>> {
    if patterns.is_empty() {
        return Ok(Vec::new());
    }
    let compiled = utils::compile_patterns(patterns)?;
    let (selected, missing_patterns) =
        filter_and_track_patterns(names, &compiled, |name, pattern| {
            if pattern.is_glob() {
                pattern.matches(name)
            } else {
                *name == pattern.source()
            }
        });
    report_missing_patterns(&missing_patterns, MissingFiles::Fail)?;
    Ok(selected.into_iter().map(|name| name.to_string()).collect())
}

/// Filter items by patterns, tracking which patterns matched.
///
/// Returns (matched_items, unmatched_patterns). Each item appears once however
/// many patterns select it, and every one of those patterns counts as found.
pub fn filter_and_track_patterns<'a, 'p, T, P>(
    items: &'a [T],
    patterns: &'p [P],
    matcher: impl Fn(&T, &P) -> bool,
) -> (Vec<&'a T>, Vec<&'p P>) {
    if patterns.is_empty() {
        return (items.iter().collect(), Vec::new());
    }

    let mut patterns_found = vec![false; patterns.len()];
    let mut filtered_items = Vec::new();

    for item in items {
        let mut selected = false;
        for (idx, pattern) in patterns.iter().enumerate() {
            if matcher(item, pattern) {
                patterns_found[idx] = true;
                selected = true;
            }
        }
        if selected {
            filtered_items.push(item);
        }
    }

    let missing_patterns = patterns
        .iter()
        .zip(patterns_found)
        .filter_map(|(pattern, found)| (!found).then_some(pattern))
        .collect();

    (filtered_items, missing_patterns)
}

// ── Utility functions ──────────────────────────────────────────────

/// Helper functions for file/path operations and pattern matching
pub mod utils {
    use super::*;
    use std::borrow::Cow;

    /// Print formatted file listing to stdout.
    /// Output goes through `print_stdout`, so a closed pipe (e.g. `| head`) silences it.
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

    /// Stream archive bytes to a same-directory temp file via the given closure,
    /// then rename it over the target, so an interrupted save cannot destroy an
    /// existing archive. Streaming keeps peak memory at one file's data instead
    /// of buffering the whole archive.
    ///
    /// The temp file is synced before the rename (and the directory after it on
    /// Unix): a rename is durable before the data it points at, so without the sync
    /// a power loss shortly after saving can leave the archive truncated. It is named per process
    /// and created exclusively, so concurrent saves of one archive cannot write
    /// into each other's temp file. On Unix it takes the replaced archive's permissions.
    pub fn write_atomically(
        path: &Path,
        write: impl FnOnce(&mut std::io::BufWriter<fs::File>) -> Result<()>,
    ) -> Result<()> {
        let file_name = path
            .file_name()
            .with_context(|| format!("Invalid archive path: {}", path.display()))?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.subsec_nanos())
            .unwrap_or_default();
        // std::process::id panics as unsupported on WASI; the timestamp and the
        // exclusive create below still keep one save out of another's temp file there.
        #[cfg(target_os = "wasi")]
        let pid = 0;
        #[cfg(not(target_os = "wasi"))]
        let pid = std::process::id();
        let mut tmp_name = std::ffi::OsString::from(".");
        tmp_name.push(file_name);
        tmp_name.push(format!(".{pid}-{nanos}.tmp"));
        let tmp_path = path.with_file_name(tmp_name);

        // Opened on its own so a failure here never removes a file this call did not create
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .with_context(|| format!("Failed to create {}", tmp_path.display()))?;

        let result = (|| -> Result<()> {
            let mut writer = std::io::BufWriter::new(file);
            write(&mut writer)?;
            let file = writer
                .into_inner()
                .map_err(|e| e.into_error())
                .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
            file.sync_all()
                .with_context(|| format!("Failed to write {}", tmp_path.display()))?;
            // Unix only: WASI cannot set permissions, and on Windows the one
            // permission bit, read-only, would make the rename below fail.
            #[cfg(unix)]
            if let Ok(existing) = fs::metadata(path) {
                file.set_permissions(existing.permissions())
                    .with_context(|| {
                        format!("Failed to set permissions on {}", tmp_path.display())
                    })?;
            }
            fs::rename(&tmp_path, path)
                .with_context(|| format!("Failed to move archive into place: {}", path.display()))
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&tmp_path);
            return Err(e);
        }

        #[cfg(unix)]
        {
            let dir = match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent,
                _ => Path::new("."),
            };
            fs::File::open(dir)
                .and_then(|dir_handle| dir_handle.sync_all())
                .with_context(|| format!("Failed to sync directory {}", dir.display()))?;
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
    pub fn read_file_slice<'a>(archive_data: &'a [u8], file: &'a FileEntry) -> Result<&'a [u8]> {
        if let Some(ref data) = file.data {
            return Ok(data);
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

        Ok(&archive_data[start..end])
    }

    /// Collect all files from a path (file or directory, recursive), reporting each
    /// symlink it skips. Validates that all filenames are ASCII-only.
    pub fn collect_files<P: AsRef<Path>>(path: P) -> Result<Vec<PathBuf>> {
        let mut files = Vec::new();
        let mut skipped = Vec::new();
        collect_files_inner(path.as_ref(), &mut files, &mut skipped)?;
        for (link, dangling) in skipped {
            if dangling {
                eprintln!("Skipping dangling symlink: {}", link.display());
            } else {
                eprintln!("Skipping symlink: {}", link.display());
            }
        }
        Ok(files)
    }

    /// Count the files `collect_files` would return, without reporting skipped
    /// symlinks: `a` counts up front and then collects again as it adds, and the
    /// warnings belong to the pass that does the adding.
    pub fn count_files<P: AsRef<Path>>(path: P) -> Result<usize> {
        let mut files = Vec::new();
        collect_files_inner(path.as_ref(), &mut files, &mut Vec::new())?;
        Ok(files.len())
    }

    /// Inner recursive worker for `collect_files`: files go to `out`, skipped
    /// symlinks to `skipped` with whether their target is missing.
    ///
    /// Validates ASCII at the leaf push site so each path is checked exactly once.
    fn collect_files_inner(
        path: &Path,
        out: &mut Vec<PathBuf>,
        skipped: &mut Vec<(PathBuf, bool)>,
    ) -> Result<()> {
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
            let dangling = !matches!(path.try_exists(), Ok(true));
            skipped.push((path.to_path_buf(), dangling));
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
            let entries = fs::read_dir(path)
                .with_context(|| format!("Failed to read directory: {}", path.display()))?;
            for entry in entries {
                let entry = entry
                    .with_context(|| format!("Failed to read directory: {}", path.display()))?;
                collect_files_inner(&entry.path(), out, skipped)?;
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

    /// A user-supplied name or glob, compiled once per command and then matched
    /// against every entry.
    ///
    /// A pattern with glob metacharacters is a glob, matched case-insensitively;
    /// one without a path separator matches the file name alone. Any other pattern
    /// matches as a case-sensitive substring, for backward compatibility.
    pub struct NamePattern {
        source: String,
        kind: PatternKind,
    }

    enum PatternKind {
        Glob {
            glob: glob::Pattern,
            whole_path: bool,
        },
        Substring,
    }

    impl NamePattern {
        /// Compile `pattern`, rejecting a malformed glob rather than quietly
        /// matching it as text.
        pub fn new(pattern: &str) -> Result<Self> {
            let kind = if contains_glob_metacharacters(pattern) {
                let normalized = pattern.replace('\\', "/");
                let glob = glob::Pattern::new(&normalized).map_err(|_| {
                    anyhow::anyhow!(
                        "Invalid glob pattern: {}",
                        normalize_path_for_display(pattern)
                    )
                })?;
                PatternKind::Glob {
                    glob,
                    whole_path: normalized.contains('/'),
                }
            } else {
                PatternKind::Substring
            };
            Ok(Self {
                source: pattern.to_string(),
                kind,
            })
        }

        /// The pattern as given, for reporting
        pub fn source(&self) -> &str {
            &self.source
        }

        pub fn is_glob(&self) -> bool {
            matches!(self.kind, PatternKind::Glob { .. })
        }

        pub fn matches(&self, file_name: &str) -> bool {
            match &self.kind {
                PatternKind::Substring => file_name.contains(self.source.as_str()),
                PatternKind::Glob { glob, whole_path } => {
                    let normalized = file_name.replace('\\', "/");
                    let target = if *whole_path {
                        normalized.as_str()
                    } else {
                        normalized.rsplit('/').next().unwrap_or(&normalized)
                    };
                    // Archive names come from DOS/Windows tooling, where case carries no
                    // meaning (entries are even sorted case-insensitively), so `*.frm`
                    // must select `A.FRM`.
                    let options = glob::MatchOptions {
                        case_sensitive: false,
                        ..glob::MatchOptions::new()
                    };
                    glob.matches_with(target, options)
                }
            }
        }
    }

    /// Read a file that is being added to an archive. Every format stores entry
    /// sizes as u32, so a larger file is refused from its metadata before reading.
    pub fn read_entry_data(file: &Path) -> Result<Vec<u8>> {
        let len = fs::metadata(file)
            .with_context(|| format!("Failed to read {}", file.display()))?
            .len();
        if len > u64::from(u32::MAX) {
            bail!(
                "{} is larger than the 4 GiB a DAT archive entry can hold",
                file.display()
            );
        }
        fs::read(file).with_context(|| format!("Failed to read {}", file.display()))
    }

    /// Normalize user patterns and compile each one
    pub fn compile_patterns(patterns: &[String]) -> Result<Vec<NamePattern>> {
        normalize_user_patterns(patterns)
            .iter()
            .map(|pattern| NamePattern::new(pattern))
            .collect()
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

    /// Why Windows would not create this path component as named, if it would not.
    ///
    /// Checked on every host for the same reason as drive prefixes: `name:stream`
    /// writes an NTFS alternate data stream, a device name such as `CON` opens the
    /// device rather than a file, and a trailing dot or space is silently stripped.
    fn windows_unsafe_component(part: &str) -> Option<&'static str> {
        if part.contains(':') {
            return Some("':' in name");
        }
        if part.ends_with('.') || part.ends_with(' ') {
            return Some("trailing dot or space in name");
        }
        let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
        let numbered_port = stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit();
        (matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered_port)
            .then_some("reserved device name")
    }

    /// Split an archive path into its safe components, rejecting every shape
    /// that would let it escape the directory it is resolved against (`..`, an
    /// absolute root, a drive prefix) or that Windows would not create as named.
    /// `.` components are dropped.
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
                    if let Some(reason) = windows_unsafe_component(part) {
                        bail!("{reason} '{part}'");
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
    /// directory - `..`, an absolute root, or a drive prefix - or onto something
    /// other than the file it names on Windows.
    ///
    /// A malicious archive could store an entry as `../../../etc/passwd` or as
    /// `\tmp\x.txt`; the second is the more dangerous shape, because `Path::join`
    /// discards the output directory rather than nesting under it.
    pub fn validate_archive_path(path: &str) -> Result<()> {
        archive_path_parts(path).map_err(|e| {
            anyhow::anyhow!(
                "Unsafe path in archive entry ({e}): {}",
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
    /// Split entries for flat extraction into the ones to write and the ones a
    /// later entry of the same file name replaces.
    ///
    /// Same-named files in different directories are common in real archives (the
    /// Fallout 1 `master.dat` holds about two thousand), so a collision is not an
    /// error. Keeping the last one matches what extracting in archive order would
    /// leave, instead of whichever parallel write happened to finish last.
    pub fn last_entry_per_flat_name<'a>(
        files: &[&'a FileEntry],
    ) -> (Vec<&'a FileEntry>, Vec<&'a FileEntry>) {
        use std::collections::HashMap;

        let mut last_index: HashMap<&str, usize> = HashMap::new();
        for (index, file) in files.iter().enumerate() {
            last_index.insert(get_filename_from_dat_path(&file.name), index);
        }
        files.iter().enumerate().fold(
            (Vec::new(), Vec::new()),
            |(mut kept, mut replaced), (index, file)| {
                if last_index[get_filename_from_dat_path(&file.name)] == index {
                    kept.push(*file);
                } else {
                    replaced.push(*file);
                }
                (kept, replaced)
            },
        )
    }

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
                std::path::Component::Prefix(_)
                | std::path::Component::RootDir
                | std::path::Component::CurDir
                | std::path::Component::ParentDir => {
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
                other @ (std::path::Component::CurDir
                | std::path::Component::ParentDir
                | std::path::Component::Normal(_)) => normalized_path.push(other.as_os_str()),
            }
        }

        normalize_path_separators(&normalized_path.to_string_lossy())
    }
}
