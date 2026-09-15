/*!
# dat3-core

Read, list, extract, add to and delete from Fallout (DAT1, DAT2) and Troika
(Arcanum, ToEE) archives.

## Entry points

- [`DatArchive`] is an open or new archive: [`open`](DatArchive::open) detects the
  format, [`new`](DatArchive::new) starts an empty one, and
  [`list`](DatArchive::list), [`extract`](DatArchive::extract),
  [`add_file`](DatArchive::add_file), [`delete`](DatArchive::delete) and
  [`save`](DatArchive::save) work on it. Adds and deletes change only the
  in-memory archive until it is saved.
- For archives and files held in memory, [`from_bytes`](DatArchive::from_bytes),
  [`entries`](DatArchive::entries), [`read`](DatArchive::read),
  [`insert`](DatArchive::insert), [`remove`](DatArchive::remove) and
  [`to_bytes`](DatArchive::to_bytes) touch no filesystem and print nothing.
- [`ArchiveFormat`] names a format.
- The options those methods take live in [`common`]: [`Selection`](common::Selection),
  [`CaseMode`](common::CaseMode), [`MissingFiles`](common::MissingFiles),
  [`ExtractionMode`](common::ExtractionMode), [`ListFormat`](common::ListFormat) and
  [`CompressionLevel`](common::CompressionLevel).

The remaining items in [`common`] and [`common::utils`] are the helpers the
formats and the `dat3` command line share. They are public so that command line
can use them, and change with it.

## Conventions

- Entry names are stored with `\` separators; every method taking a name or
  pattern also accepts `/`.
- A pattern with glob metacharacters (`*`, `?`, `[`) is a glob, matched against the
  whole name if it contains a separator and against the file name otherwise. Any
  other pattern matches as a substring. See [`NamePattern`](common::utils::NamePattern).
- [`CaseMode::Insensitive`](common::CaseMode::Insensitive) matches regardless of
  case and shows, extracts and adds names in lowercase, except names in the archive
  that differ only in case, which keep their stored case.
  [`CaseMode::Sensitive`](common::CaseMode::Sensitive) uses names exactly as stored.
- Errors are [`anyhow::Error`], worded for a person to read.
- The path-based operations print as the `dat3` command line does: listings and
  progress to stdout, warnings and patterns that matched nothing to stderr.
- [`DatArchive::open`] and [`DatArchive::from_bytes`] hold the whole archive in memory.

## Example

```no_run
use dat3_core::common::{CaseMode, CompressionLevel, ExtractionMode, MissingFiles, Selection};
use dat3_core::{ArchiveFormat, DatArchive};

// Build a Fallout 2 archive from the `art` directory
let mut archive = DatArchive::new(ArchiveFormat::Dat2);
archive.add_file("art", CompressionLevel::new(9)?, None, None, CaseMode::Insensitive)?;
archive.save("patch000.dat")?;

// Open it again and extract the FRM files, keeping their directories
let archive = DatArchive::open("patch000.dat")?;
let patterns = ["*.frm".to_string()];
let selection = Selection {
    patterns: &patterns,
    on_missing: MissingFiles::Fail,
    case: CaseMode::Insensitive,
};
archive.extract("out", ExtractionMode::PreserveStructure, &selection)?;
# anyhow::Ok(())
```
*/

#![warn(missing_docs)]

mod arcanum; // Arcanum (Troika) DAT format implementation
pub mod archive; // ArchiveFormat and the unified DatArchive interface
pub mod common; // Shared types, archive operations, and path utilities
mod dat1; // Fallout 1 DAT format implementation
mod dat2; // Fallout 2 DAT format implementation
mod lzss; // LZSS decompression for DAT1 files
mod toee; // The Temple of Elemental Evil (Troika) DAT format implementation

#[cfg(test)]
mod common_tests;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support; // Self-cleaning scratch paths for the test modules

pub use archive::{ArchiveFormat, DatArchive};
