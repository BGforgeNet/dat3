/*!
# Archive formats

`ArchiveFormat` names the supported formats, and `DatArchive` wraps their
implementations behind one interface so callers don't need to know which
format they're working with. Kept out of `common`, which the format modules
build on, so module dependencies run one way.
*/

use anyhow::{Context, Result, bail};
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::arcanum::{self, ArcanumArchive};
use crate::common::{
    self, CaseMode, CompressionLevel, ExtractionMode, ListFormat, Selection, utils,
};
use crate::dat1::Dat1Archive;
use crate::dat2::Dat2Archive;
use crate::toee::{self, ToeeArchive};

// DAT1 format detection: big-endian header, no signature to key on
const DAT1_MAX_DIRECTORIES: u32 = 1000;

/// A supported archive format, as selected by `a --format` or `.bgforge.yml`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum ArchiveFormat {
    /// Fallout 1 (big-endian, LZSS; created uncompressed)
    Dat1,
    /// Fallout 2 (little-endian, zlib) - the default for new archives
    Dat2,
    /// Arcanum (little-endian, zlib)
    Arcanum,
    /// The Temple of Elemental Evil (hierarchical Troika DAT, zlib)
    Toee,
}

impl ArchiveFormat {
    /// Every format
    pub const ALL: [Self; 4] = [Self::Dat1, Self::Dat2, Self::Arcanum, Self::Toee];

    /// The format whose [`arg_name`](Self::arg_name) is `name`
    pub fn from_arg_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|format| format.arg_name() == name)
    }

    /// The value as typed on the command line, for error messages
    pub fn arg_name(self) -> &'static str {
        match self {
            Self::Dat1 => "dat1",
            Self::Dat2 => "dat2",
            Self::Arcanum => "arcanum",
            Self::Toee => "toee",
        }
    }

    /// Human-readable format name for messages
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Dat1 => "DAT1",
            Self::Dat2 => "DAT2",
            Self::Arcanum => "Arcanum",
            Self::Toee => "ToEE",
        }
    }
}

/// Unified interface for DAT1, DAT2, Arcanum, and ToEE archives.
///
/// Uses an enum instead of trait objects because the set of known formats
/// is small and fixed - this gives us static dispatch, exhaustive matching,
/// and no heap allocation for the wrapper.
///
/// **Memory**: The entire archive is loaded into memory on open, whatever the
/// format. Fallout archives typically stay under ~200MB; retail Arcanum
/// archives run considerably larger and are held in RAM the same way.
///
/// ```no_run
/// use dat3_core::{ArchiveFormat, DatArchive};
///
/// let archive = DatArchive::open("master.dat")?; // auto-detects format
/// let dat1 = DatArchive::new(ArchiveFormat::Dat1); // create new DAT1
/// # anyhow::Ok(())
/// ```
#[derive(Debug)]
pub enum DatArchive {
    /// Fallout 1 format (big-endian, hierarchical dirs, LZSS compression)
    Dat1(Dat1Archive),
    /// Fallout 2 format (little-endian, flat file list, zlib compression)
    Dat2(Dat2Archive),
    /// Arcanum format (little-endian, flat entry table, zlib compression)
    Arcanum(ArcanumArchive),
    /// ToEE format (little-endian, hierarchical entry table, zlib compression)
    Toee(ToeeArchive),
}

impl DatArchive {
    /// Open an existing DAT archive, auto-detecting the format.
    ///
    /// Reads the whole file into memory. DAT2 has no signature, so a file that is
    /// none of the other formats is parsed as DAT2 and fails there if it is not one.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let data = fs::read(&path)
            .with_context(|| format!("Failed to read DAT file: {}", path.as_ref().display()))?;
        Self::from_bytes(data)
    }

    /// Parse an archive held in memory, auto-detecting the format as [`open`](Self::open) does
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        // Troika formats share a real magic and go first. ToEE's hierarchical
        // entry-table size distinguishes it from Arcanum's flat table. DAT1 is
        // a header heuristic and DAT2 (no signature at all) is the fallback.
        if toee::is_toee_format(&data) {
            Ok(Self::Toee(ToeeArchive::from_bytes(data)?))
        } else if arcanum::is_arcanum_format(&data) {
            Ok(Self::Arcanum(ArcanumArchive::from_bytes(data)?))
        } else if Self::is_dat1_format(&data) {
            Ok(Self::Dat1(Dat1Archive::from_bytes(data)?))
        } else {
            Ok(Self::Dat2(Dat2Archive::from_bytes(data)?))
        }
    }

    /// Create a new empty archive of the given format. Nothing is written until [`save`](Self::save).
    pub fn new(format: ArchiveFormat) -> Self {
        match format {
            ArchiveFormat::Dat1 => Self::Dat1(Dat1Archive::new()),
            ArchiveFormat::Dat2 => Self::Dat2(Dat2Archive::new()),
            ArchiveFormat::Arcanum => Self::Arcanum(ArcanumArchive::new()),
            ArchiveFormat::Toee => Self::Toee(ToeeArchive::new()),
        }
    }

    /// The format this archive is stored in
    pub fn format(&self) -> ArchiveFormat {
        match self {
            Self::Dat1(_) => ArchiveFormat::Dat1,
            Self::Dat2(_) => ArchiveFormat::Dat2,
            Self::Arcanum(_) => ArchiveFormat::Arcanum,
            Self::Toee(_) => ArchiveFormat::Toee,
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

    /// Print the entries `selection` picks to stdout, as columns or a JSON array.
    ///
    /// Patterns that match nothing are printed to stderr after the listing, and
    /// fail the call when `selection.on_missing` is [`MissingFiles::Fail`](crate::common::MissingFiles::Fail).
    pub fn list(&self, selection: &Selection, format: ListFormat) -> Result<()> {
        match self {
            Self::Dat1(a) => a.list(selection, format),
            Self::Dat2(a) => a.list(selection, format),
            Self::Arcanum(a) => a.list(selection, format),
            Self::Toee(a) => a.list(selection, format),
        }
    }

    /// Extract the entries `selection` picks into `output_dir`, in parallel.
    ///
    /// Every selected name is checked before anything is written: a name that is
    /// unsafe to create, or a pattern that matches nothing under
    /// [`MissingFiles::Fail`](crate::common::MissingFiles::Fail), fails with no files written. Names are
    /// written as `selection.case` shows them. Progress goes to stdout; in
    /// [`ExtractionMode::Flat`] entries whose file name a later entry reuses are
    /// skipped with a warning on stderr.
    pub fn extract<P: AsRef<Path>>(
        &self,
        output_dir: P,
        mode: ExtractionMode,
        selection: &Selection,
    ) -> Result<()> {
        let output_dir = output_dir.as_ref();
        match self {
            Self::Dat1(a) => a.extract(output_dir, mode, selection),
            Self::Dat2(a) => a.extract(output_dir, mode, selection),
            Self::Arcanum(a) => a.extract(output_dir, mode, selection),
            Self::Toee(a) => a.extract(output_dir, mode, selection),
        }
    }

    /// Add a file, or a directory recursively (symlinks are skipped), in memory.
    ///
    /// The entry name comes from `file_path`:
    /// - with `source_root`, its path relative to that root, which must prefix it
    ///   (pass both canonicalized);
    /// - otherwise the path as given without a leading `./`, or with `target_dir`
    ///   just the file's own name, or a directory's name and its contents.
    ///
    /// `target_dir` is prefixed to the name in either case, and a name the archive
    /// cannot hold fails (see [`common::utils::validate_add_archive_path`]). An entry whose
    /// name equals an existing one, compared as `case` says, replaces it; under
    /// [`CaseMode::Insensitive`] new names are stored in lowercase. DAT1 stores
    /// entries uncompressed whatever `compression` says. Prints each added name to stdout.
    pub fn add_file<P: AsRef<Path>>(
        &mut self,
        file_path: P,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        source_root: Option<&Path>,
        case: CaseMode,
    ) -> Result<()> {
        let path = file_path.as_ref();
        match self {
            Self::Dat1(a) => a.add_file(path, compression, target_dir, source_root, case),
            Self::Dat2(a) => a.add_file(path, compression, target_dir, source_root, case),
            Self::Arcanum(a) => a.add_file(path, compression, target_dir, source_root, case),
            Self::Toee(a) => a.add_file(path, compression, target_dir, source_root, case),
        }
    }

    /// Names of every file entry, as stored: backslash-separated, in their stored case
    pub fn entry_names(&self) -> Vec<String> {
        self.file_entries()
            .into_iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    /// Every file entry with its sizes, names as stored (see [`entry_names`](Self::entry_names))
    pub fn entries(&self) -> Vec<Entry> {
        self.file_entries()
            .into_iter()
            .map(|file| Entry {
                name: file.name.clone(),
                size: file.size,
                packed_size: file.packed_size,
                compressed: file.compressed,
            })
            .collect()
    }

    fn file_entries(&self) -> Vec<&common::FileEntry> {
        match self {
            Self::Dat1(a) => a.entries(),
            Self::Dat2(a) => a.entries(),
            Self::Arcanum(a) => a.entries(),
            Self::Toee(a) => a.entries(),
        }
    }

    /// The decompressed contents of the entry `name` refers to.
    ///
    /// `name` may use `/` or `\`. The entry with exactly that name wins; failing
    /// that, under [`CaseMode::Insensitive`], the one entry whose name equals it
    /// ignoring case. Several such entries, or none, is an error.
    pub fn read(&self, name: &str, case: CaseMode) -> Result<Vec<u8>> {
        let files = self.file_entries();
        let index = common::find_stored_index(files.iter().map(|f| f.name.as_str()), name, case)?
            .with_context(|| {
            format!(
                "File not found: {}",
                utils::normalize_path_for_display(name)
            )
        })?;
        let file = files[index];
        let contents = match self {
            Self::Dat1(a) => a.contents(file)?,
            Self::Dat2(a) => a.contents(file)?,
            Self::Arcanum(a) => a.contents(file)?,
            Self::Toee(a) => a.contents(file)?,
        };
        Ok(contents.into_owned())
    }

    /// Add `data` as the entry `name` (`/` or `\` separated), in memory.
    ///
    /// The name is stored as given, and replaces any entry equal to it as `case`
    /// compares. A name the archive cannot hold fails (see
    /// [`utils::stored_name_for_insert`]), as does data over 4 GiB. The zlib
    /// formats compress at `compression` when that saves space; DAT1 stores it
    /// uncompressed. Prints nothing.
    pub fn insert(
        &mut self,
        name: &str,
        data: Vec<u8>,
        compression: CompressionLevel,
        case: CaseMode,
    ) -> Result<()> {
        let stored = utils::stored_name_for_insert(name)?;
        if u32::try_from(data.len()).is_err() {
            bail!(
                "{} is larger than the 4 GiB a DAT archive entry can hold",
                utils::normalize_path_for_display(&stored)
            );
        }
        match self {
            Self::Dat1(a) => {
                let mut entry = common::FileEntry::with_data(stored, data, false);
                entry.size = entry.packed_size;
                a.insert_entry(entry, case);
            }
            Self::Dat2(a) => a.insert_entry(common::zlib_entry(stored, data, compression)?, case),
            Self::Arcanum(a) => {
                a.insert_entry(common::zlib_entry(stored, data, compression)?, case)
            }
            Self::Toee(a) => a.insert_entry(common::zlib_entry(stored, data, compression)?, case),
        }
        Ok(())
    }

    /// Remove the entry `name` refers to, found as [`read`](Self::read) finds it,
    /// in memory. Returns whether an entry was removed. Prints nothing.
    pub fn remove(&mut self, name: &str, case: CaseMode) -> Result<bool> {
        let names = self.entry_names();
        let Some(index) = common::find_stored_index(names.iter().map(String::as_str), name, case)?
        else {
            return Ok(false);
        };
        Ok(self.remove_entry(&names[index]))
    }

    fn remove_entry(&mut self, stored_name: &str) -> bool {
        match self {
            Self::Dat1(a) => a.remove_entry(stored_name),
            Self::Dat2(a) => a.remove_entry(stored_name),
            Self::Arcanum(a) => a.remove_entry(stored_name),
            Self::Toee(a) => a.remove_entry(stored_name),
        }
    }

    /// Delete every entry `patterns` select, in memory.
    ///
    /// A glob deletes every entry it matches; a plain name deletes only the entry
    /// with that whole name. If any pattern matches nothing, nothing is deleted
    /// (see [`resolve_delete_targets`](crate::common::resolve_delete_targets)).
    /// Prints each deleted name to stdout.
    pub fn delete(&mut self, patterns: &[String], case: CaseMode) -> Result<()> {
        let names = self.entry_names();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        for target in common::resolve_delete_targets(&names, patterns, case)? {
            self.delete_file(&target)?;
        }
        Ok(())
    }

    /// Delete the entry named exactly `file_name` (`/` or `\` separated), in memory.
    /// Prints the deleted name to stdout.
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        if !self.remove_entry(file_name) {
            bail!(
                "File not found: {}",
                utils::normalize_path_for_display(file_name)
            );
        }
        let normalized = utils::normalize_user_path(file_name);
        common::print_stdout(format_args!(
            "Deleting: {}",
            utils::normalize_path_for_display(&normalized)
        ));
        Ok(())
    }

    /// Write the archive to `path`, replacing any file there only once the new one
    /// is complete and flushed to disk.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        match self {
            Self::Dat1(a) => a.save(path.as_ref()),
            Self::Dat2(a) => a.save(path.as_ref()),
            Self::Arcanum(a) => a.save(path.as_ref()),
            Self::Toee(a) => a.save(path.as_ref()),
        }
    }

    /// Write the archive to `out`, byte for byte what [`save`](Self::save) writes
    pub fn write_to(&self, out: &mut dyn Write) -> Result<()> {
        match self {
            Self::Dat1(a) => a.write_to(out),
            Self::Dat2(a) => a.write_to(out),
            Self::Arcanum(a) => a.write_to(out),
            Self::Toee(a) => a.write_to(out),
        }
    }

    /// The archive as it would be saved
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.write_to(&mut out)?;
        Ok(out)
    }
}

/// One file entry's name and sizes, as [`DatArchive::entries`] reports it
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    /// Name as stored: backslash-separated, in its stored case
    pub name: String,
    /// Size of the contents in bytes
    pub size: u32,
    /// Size as stored in the archive, compressed or not
    pub packed_size: u32,
    /// Whether the stored data is compressed
    pub compressed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchPath;

    const ALL_FORMATS: [ArchiveFormat; 4] = ArchiveFormat::ALL;

    #[test]
    fn from_arg_name_names_every_format_and_nothing_else() {
        for format in ALL_FORMATS {
            assert_eq!(
                ArchiveFormat::from_arg_name(format.arg_name()),
                Some(format)
            );
        }
        assert_eq!(ArchiveFormat::from_arg_name("zip"), None);
        assert_eq!(ArchiveFormat::from_arg_name("DAT2"), None);
    }

    /// A new archive holding one small file per name (`/`-separated), stored in
    /// exactly the case given
    fn archive_with(format: ArchiveFormat, names: &[&str]) -> DatArchive {
        let mut archive = DatArchive::new(format);
        add_names(&mut archive, names, CaseMode::Sensitive);
        archive
    }

    /// Add one small file per name, as `a` would under `case`
    fn add_names(archive: &mut DatArchive, names: &[&str], case: CaseMode) {
        let src = ScratchPath::dir("archive_names_src");
        for name in names {
            let path = src.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, name.as_bytes()).unwrap();
            archive
                .add_file(
                    &path,
                    CompressionLevel::new(0).unwrap(),
                    None,
                    Some(&src),
                    case,
                )
                .unwrap();
        }
    }

    /// Save and reopen, so assertions see what the format actually stored
    fn reopened(archive: &DatArchive) -> DatArchive {
        let path = ScratchPath::new("archive_case_roundtrip");
        archive.save(&path).unwrap();
        DatArchive::open(&path).unwrap()
    }

    fn all_entries(case: CaseMode) -> Selection<'static> {
        Selection {
            patterns: &[],
            on_missing: common::MissingFiles::Fail,
            case,
        }
    }

    #[test]
    fn by_default_new_entries_are_stored_in_lowercase() {
        for format in ALL_FORMATS {
            let mut archive = DatArchive::new(format);
            add_names(&mut archive, &["Art/Hero.FRM"], CaseMode::Insensitive);
            assert_eq!(
                sorted_names(&reopened(&archive)),
                ["art\\hero.frm"],
                "{format:?}"
            );
        }
    }

    #[test]
    fn with_case_sensitive_new_entries_keep_their_case() {
        for format in ALL_FORMATS {
            let archive = archive_with(format, &["Art/Hero.FRM"]);
            assert_eq!(
                sorted_names(&reopened(&archive)),
                ["Art\\Hero.FRM"],
                "{format:?}"
            );
        }
    }

    #[test]
    fn by_default_extraction_writes_lowercase_paths() {
        for format in ALL_FORMATS {
            let archive = reopened(&archive_with(format, &["ART/HERO.FRM"]));
            let out = ScratchPath::dir("archive_case_extract");
            archive
                .extract(
                    &out,
                    ExtractionMode::PreserveStructure,
                    &all_entries(CaseMode::Insensitive),
                )
                .unwrap();
            assert!(out.join("art").join("hero.frm").is_file(), "{format:?}");
            assert!(!out.join("ART").exists(), "{format:?}");
        }
    }

    #[test]
    fn with_case_sensitive_extraction_keeps_stored_paths() {
        for format in ALL_FORMATS {
            let archive = reopened(&archive_with(format, &["ART/HERO.FRM"]));
            let out = ScratchPath::dir("archive_case_extract_exact");
            archive
                .extract(
                    &out,
                    ExtractionMode::PreserveStructure,
                    &all_entries(CaseMode::Sensitive),
                )
                .unwrap();
            assert!(out.join("ART").join("HERO.FRM").is_file(), "{format:?}");
        }
    }

    /// Re-adding a file under another case replaces the entry rather than
    /// keeping two names that differ only in case.
    #[test]
    fn by_default_adding_replaces_an_entry_differing_only_in_case() {
        for format in ALL_FORMATS {
            let mut archive = reopened(&archive_with(format, &["ART/HERO.FRM", "ART/OTHER.FRM"]));
            add_names(&mut archive, &["art/hero.frm"], CaseMode::Insensitive);
            // DAT1 and ToEE keep one stored spelling per directory, so the new
            // lowercase file sits under the existing ART; the flat formats store
            // the whole lowercase path.
            let expected = match format {
                ArchiveFormat::Dat1 | ArchiveFormat::Toee => ["ART\\OTHER.FRM", "ART\\hero.frm"],
                ArchiveFormat::Dat2 | ArchiveFormat::Arcanum => ["ART\\OTHER.FRM", "art\\hero.frm"],
            };
            assert_eq!(sorted_names(&reopened(&archive)), expected, "{format:?}");
        }
    }

    #[test]
    fn by_default_delete_ignores_case() {
        for format in ALL_FORMATS {
            let mut archive = reopened(&archive_with(format, &["ART/HERO.FRM", "ART/OTHER.FRM"]));
            archive
                .delete(&["art/hero.frm".to_string()], CaseMode::Insensitive)
                .unwrap();
            assert_eq!(sorted_names(&archive), ["ART\\OTHER.FRM"], "{format:?}");
        }
    }

    #[test]
    fn with_case_sensitive_delete_needs_the_exact_case() {
        for format in ALL_FORMATS {
            let mut archive = reopened(&archive_with(format, &["ART/HERO.FRM"]));
            assert!(
                archive
                    .delete(&["art/hero.frm".to_string()], CaseMode::Sensitive)
                    .is_err(),
                "{format:?}"
            );
        }
    }

    /// DAT1 stores one name per directory, so a file added under a differently
    /// cased directory joins the stored one instead of creating a second.
    #[test]
    fn dat1_adding_into_a_directory_stored_in_another_case_round_trips() {
        let mut archive = reopened(&archive_with(ArchiveFormat::Dat1, &["ART/HERO.FRM"]));
        add_names(&mut archive, &["art/new.frm"], CaseMode::Insensitive);
        let archive = reopened(&archive);
        assert_eq!(sorted_names(&archive), ["ART\\HERO.FRM", "ART\\new.frm"]);

        let out = ScratchPath::dir("dat1_case_dir");
        archive
            .extract(
                &out,
                ExtractionMode::PreserveStructure,
                &all_entries(CaseMode::Insensitive),
            )
            .unwrap();
        assert_eq!(
            std::fs::read(out.join("art").join("new.frm")).unwrap(),
            b"art/new.frm"
        );
    }

    fn sorted_names(archive: &DatArchive) -> Vec<String> {
        let mut names = archive.entry_names();
        names.sort();
        names
    }

    #[test]
    fn a_glob_deletes_every_matching_entry_and_nothing_else() {
        for format in ALL_FORMATS {
            let mut archive = archive_with(
                format,
                &["ART/A.FRM", "ART/B.FRM", "ART/A.TXT", "TEXT/A.FRM"],
            );
            archive
                .delete(&["ART/*.FRM".to_string()], CaseMode::Sensitive)
                .unwrap();
            assert_eq!(
                sorted_names(&archive),
                ["ART\\A.TXT", "TEXT\\A.FRM"],
                "{format:?}"
            );
        }
    }

    #[test]
    fn a_plain_name_deletes_only_the_entry_with_that_exact_name() {
        for format in ALL_FORMATS {
            let mut archive = archive_with(format, &["DATA/A.TXT", "DATA/BIGA.TXT"]);
            archive
                .delete(&["DATA/A.TXT".to_string()], CaseMode::Sensitive)
                .unwrap();
            assert_eq!(sorted_names(&archive), ["DATA\\BIGA.TXT"], "{format:?}");
        }
    }

    #[test]
    fn an_unmatched_pattern_fails_before_anything_is_deleted() {
        for format in ALL_FORMATS {
            let mut archive = archive_with(format, &["DATA/A.TXT", "DATA/B.TXT"]);
            let err = archive
                .delete(
                    &["DATA/A.TXT".to_string(), "*.ZZZ".to_string()],
                    CaseMode::Sensitive,
                )
                .unwrap_err();
            assert_eq!(
                err.to_string(),
                "Some requested files were not found",
                "{format:?}"
            );
            assert_eq!(
                sorted_names(&archive),
                ["DATA\\A.TXT", "DATA\\B.TXT"],
                "{format:?}"
            );
        }
    }

    // -- In-memory API --

    fn level(n: u8) -> CompressionLevel {
        CompressionLevel::new(n).unwrap()
    }

    /// Serialize and parse again, without touching the filesystem
    fn through_bytes(archive: &DatArchive) -> DatArchive {
        DatArchive::from_bytes(archive.to_bytes().unwrap()).unwrap()
    }

    #[test]
    fn inserted_entries_survive_to_bytes_and_from_bytes() {
        let compressible = b"frame ".repeat(500);
        for format in ALL_FORMATS {
            let mut archive = DatArchive::new(format);
            archive
                .insert(
                    "art/Hero.FRM",
                    compressible.clone(),
                    level(9),
                    CaseMode::Insensitive,
                )
                .unwrap();
            archive
                .insert(
                    "README.TXT",
                    b"hi".to_vec(),
                    level(0),
                    CaseMode::Insensitive,
                )
                .unwrap();

            let parsed = through_bytes(&archive);
            assert_eq!(parsed.format(), format);
            assert_eq!(
                sorted_names(&parsed),
                ["README.TXT", "art\\Hero.FRM"],
                "{format:?}"
            );
            for opened in [&archive, &parsed] {
                assert_eq!(
                    opened.read("art/Hero.FRM", CaseMode::Insensitive).unwrap(),
                    compressible,
                    "{format:?}"
                );
                assert_eq!(
                    opened.read("README.TXT", CaseMode::Insensitive).unwrap(),
                    b"hi"
                );
            }
        }
    }

    #[test]
    fn to_bytes_matches_what_save_writes() {
        for format in ALL_FORMATS {
            let archive = archive_with(format, &["ART/HERO.FRM", "TEXT/A.TXT"]);
            let path = ScratchPath::new("archive_to_bytes");
            archive.save(&path).unwrap();
            assert_eq!(
                archive.to_bytes().unwrap(),
                std::fs::read(&path).unwrap(),
                "{format:?}"
            );
        }
    }

    #[test]
    fn entries_report_sizes_and_compression() {
        let compressible = b"frame ".repeat(500);
        for format in ALL_FORMATS {
            let mut archive = DatArchive::new(format);
            archive
                .insert(
                    "a.frm",
                    compressible.clone(),
                    level(9),
                    CaseMode::Insensitive,
                )
                .unwrap();
            let entries = through_bytes(&archive).entries();
            assert_eq!(entries.len(), 1, "{format:?}");
            let entry = &entries[0];
            assert_eq!(entry.name, "a.frm");
            assert_eq!(entry.size as usize, compressible.len(), "{format:?}");
            // DAT1 writing is uncompressed; the zlib formats keep compression that saves space
            match format {
                ArchiveFormat::Dat1 => {
                    assert!(!entry.compressed);
                    assert_eq!(entry.packed_size, entry.size);
                }
                ArchiveFormat::Dat2 | ArchiveFormat::Arcanum | ArchiveFormat::Toee => {
                    assert!(entry.compressed, "{format:?}");
                    assert!(entry.packed_size < entry.size, "{format:?}");
                }
            }
        }
    }

    #[test]
    fn insert_replaces_a_name_as_case_compares_and_stores_the_name_as_given() {
        for format in ALL_FORMATS {
            let mut archive = DatArchive::new(format);
            archive
                .insert(
                    "Data/A.txt",
                    b"old".to_vec(),
                    level(0),
                    CaseMode::Insensitive,
                )
                .unwrap();
            archive
                .insert(
                    "data\\A.TXT",
                    b"new".to_vec(),
                    level(0),
                    CaseMode::Insensitive,
                )
                .unwrap();
            let parsed = through_bytes(&archive);
            assert_eq!(parsed.entry_names().len(), 1, "{format:?}");
            assert_eq!(
                parsed.read("DATA/A.TXT", CaseMode::Insensitive).unwrap(),
                b"new"
            );

            archive
                .insert(
                    "data/a.txt",
                    b"twin".to_vec(),
                    level(0),
                    CaseMode::Sensitive,
                )
                .unwrap();
            assert_eq!(archive.entry_names().len(), 2, "{format:?}");
        }
    }

    #[test]
    fn insert_refuses_names_an_archive_cannot_hold() {
        let too_long = "a".repeat(common::MAX_PATH_BYTES + 1);
        for format in ALL_FORMATS {
            let mut archive = DatArchive::new(format);
            for name in [
                "",
                "../escape.txt",
                "/abs.txt",
                "dir/con.txt",
                "a:b.txt",
                too_long.as_str(),
            ] {
                assert!(
                    archive
                        .insert(name, b"x".to_vec(), level(0), CaseMode::Insensitive)
                        .is_err(),
                    "{format:?} accepted {name:?}"
                );
            }
            assert!(archive.entry_names().is_empty(), "{format:?}");
        }
    }

    #[test]
    fn read_takes_the_exact_name_first_then_any_case() {
        for format in [ArchiveFormat::Dat2, ArchiveFormat::Arcanum] {
            let mut archive = DatArchive::new(format);
            archive
                .insert(
                    "README.TXT",
                    b"upper".to_vec(),
                    level(0),
                    CaseMode::Sensitive,
                )
                .unwrap();
            archive
                .insert(
                    "readme.txt",
                    b"lower".to_vec(),
                    level(0),
                    CaseMode::Sensitive,
                )
                .unwrap();
            archive
                .insert(
                    "Other.txt",
                    b"other".to_vec(),
                    level(0),
                    CaseMode::Sensitive,
                )
                .unwrap();
            let archive = through_bytes(&archive);
            let case = CaseMode::Insensitive;

            assert_eq!(archive.read("README.TXT", case).unwrap(), b"upper");
            assert_eq!(archive.read("readme.txt", case).unwrap(), b"lower");
            assert_eq!(archive.read("OTHER.TXT", case).unwrap(), b"other");
            let ambiguous = archive.read("Readme.txt", case).unwrap_err().to_string();
            assert!(ambiguous.contains("README.TXT"), "{format:?}: {ambiguous}");
            let missing = archive.read("missing.txt", case).unwrap_err().to_string();
            assert!(missing.contains("not found"), "{format:?}: {missing}");
            assert!(archive.read("OTHER.TXT", CaseMode::Sensitive).is_err());
        }
    }

    #[test]
    fn remove_reports_whether_an_entry_was_removed() {
        for format in ALL_FORMATS {
            let mut archive =
                through_bytes(&archive_with(format, &["ART/HERO.FRM", "ART/OTHER.FRM"]));
            assert!(
                archive
                    .remove("art/hero.frm", CaseMode::Insensitive)
                    .unwrap(),
                "{format:?}"
            );
            assert!(
                !archive
                    .remove("art/hero.frm", CaseMode::Insensitive)
                    .unwrap(),
                "{format:?}"
            );
            assert_eq!(
                sorted_names(&through_bytes(&archive)),
                ["ART\\OTHER.FRM"],
                "{format:?}"
            );
        }
    }

    #[test]
    fn from_bytes_rejects_data_that_is_no_archive() {
        assert!(DatArchive::from_bytes(b"not an archive".to_vec()).is_err());
    }
}
