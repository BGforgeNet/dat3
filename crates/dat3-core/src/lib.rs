/*!
# dat3-core

Read, list, extract, add to and delete from Fallout (DAT1, DAT2) and Troika
(Arcanum, ToEE) archives. [`DatArchive`] detects the format on open.
*/

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
