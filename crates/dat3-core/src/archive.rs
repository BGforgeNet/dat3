/*!
# Archive formats

`ArchiveFormat` names the supported formats, and `DatArchive` wraps their
implementations behind one interface so callers don't need to know which
format they're working with. Kept out of `common`, which the format modules
build on, so module dependencies run one way.
*/

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

use crate::arcanum::{self, ArcanumArchive};
use crate::common::{self, CaseMode, CompressionLevel, ExtractionMode, ListFormat, Selection};
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
        let entries = match self {
            Self::Dat1(a) => a.entries(),
            Self::Dat2(a) => a.entries(),
            Self::Arcanum(a) => a.entries(),
            Self::Toee(a) => a.entries(),
        };
        entries
            .into_iter()
            .map(|entry| entry.name.clone())
            .collect()
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
        match self {
            Self::Dat1(a) => a.delete_file(file_name),
            Self::Dat2(a) => a.delete_file(file_name),
            Self::Arcanum(a) => a.delete_file(file_name),
            Self::Toee(a) => a.delete_file(file_name),
        }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchPath;

    const ALL_FORMATS: [ArchiveFormat; 4] = [
        ArchiveFormat::Dat1,
        ArchiveFormat::Dat2,
        ArchiveFormat::Arcanum,
        ArchiveFormat::Toee,
    ];

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
}
