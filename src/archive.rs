/*!
# Archive formats

`ArchiveFormat` names the supported formats, and `DatArchive` wraps their
implementations behind one interface so callers don't need to know which
format they're working with. Kept out of `common`, which the format modules
build on, so module dependencies run one way.
*/

use anyhow::{Context, Result};
use clap::ValueEnum;
use std::fs;
use std::path::Path;

use crate::arcanum::{self, ArcanumArchive};
use crate::common::{CompressionLevel, ExtractionMode, ListFormat, MissingFiles};
use crate::dat1::Dat1Archive;
use crate::dat2::Dat2Archive;
use crate::toee::{self, ToeeArchive};

// DAT1 format detection: big-endian header, no signature to key on
const DAT1_MAX_DIRECTORIES: u32 = 1000;

/// A supported archive format, as selected by `a --format` or `.bgforge.yml`
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
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
/// ```ignore
/// let archive = DatArchive::open("master.dat")?;         // auto-detects format
/// let dat1 = DatArchive::new(ArchiveFormat::Dat1);       // create new DAT1
/// ```
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
    /// Open an existing DAT archive, auto-detecting the format
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

    /// Create a new empty archive of the given format
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

    /// List files in the archive (all or filtered by patterns)
    pub fn list(
        &self,
        files: &[String],
        format: ListFormat,
        on_missing: MissingFiles,
    ) -> Result<()> {
        match self {
            Self::Dat1(a) => a.list(files, format, on_missing),
            Self::Dat2(a) => a.list(files, format, on_missing),
            Self::Arcanum(a) => a.list(files, format, on_missing),
            Self::Toee(a) => a.list(files, format, on_missing),
        }
    }

    /// Extract files from the archive
    pub fn extract<P: AsRef<Path>>(
        &self,
        output_dir: P,
        files: &[String],
        mode: ExtractionMode,
        on_missing: MissingFiles,
    ) -> Result<()> {
        match self {
            Self::Dat1(a) => a.extract(output_dir.as_ref(), files, mode, on_missing),
            Self::Dat2(a) => a.extract(output_dir.as_ref(), files, mode, on_missing),
            Self::Arcanum(a) => a.extract(output_dir.as_ref(), files, mode, on_missing),
            Self::Toee(a) => a.extract(output_dir.as_ref(), files, mode, on_missing),
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
            Self::Toee(a) => a.add_file(file_path.as_ref(), compression, target_dir, source_root),
        }
    }

    /// Delete a file from the archive
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        match self {
            Self::Dat1(a) => a.delete_file(file_name),
            Self::Dat2(a) => a.delete_file(file_name),
            Self::Arcanum(a) => a.delete_file(file_name),
            Self::Toee(a) => a.delete_file(file_name),
        }
    }

    /// Save the archive to a file
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        match self {
            Self::Dat1(a) => a.save(path.as_ref()),
            Self::Dat2(a) => a.save(path.as_ref()),
            Self::Arcanum(a) => a.save(path.as_ref()),
            Self::Toee(a) => a.save(path.as_ref()),
        }
    }
}
