/*!
# Arcanum DAT Archive Format

Archive format of Arcanum: Of Steamworks and Magick Obscura (Troika Games).
The on-disk magic decodes to "DAT1", Troika's internal name for it, but the
format is unrelated to Fallout 1's DAT1 - structurally it is a DAT2 sibling:
little-endian, flat entry table at the end of the file, zlib compression.

## File layout:
1. File data (all files concatenated, raw or zlib streams)
2. Table marker: u32 absolute offset of the entry table
3. Entry table: u32 entry count, then the entries (files and directories)
4. Footer (28 bytes): 16-byte GUID, "1TAD" magic, u32 total filename bytes,
   u32 distance from end of file back to the entry table start
*/

use anyhow::{bail, Context, Result};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use deku::prelude::*;
use std::io::{Cursor, Write};
use std::path::Path;

use crate::common::{self, utils, CompressionLevel, ExtractionMode, FileEntry};

/// Size of the trailing footer in bytes
const FOOTER_SIZE: usize = 28;

/// On-disk magic, at 12 bytes before end of file ("DAT1" read as a
/// little-endian u32)
const MAGIC: [u8; 4] = *b"1TAD";

/// Entry flag: data stored uncompressed. The parser keys only on FLAG_ZLIB
/// and FLAG_DIR (lenient toward unknown bits).
const FLAG_RAW: u32 = 0x1;
/// Entry flag: data stored as a zlib stream
const FLAG_ZLIB: u32 = 0x2;
/// Entry flag: directory entry (no data, sizes and offset are zero)
const FLAG_DIR: u32 = 0x400;

/// 28-byte footer at the end of every Arcanum DAT file
#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "little")]
struct ArcanumFooter {
    /// Written as a random GUID by the original tools; not known to be read
    guid: [u8; 16],
    magic: [u8; 4],
    /// Total bytes of all name fields in the entry table (incl. NULs)
    filename_total_bytes: u32,
    /// Distance from end of file back to the entry table's count field
    table_from_end: u32,
}

/// File or directory entry as stored in the Arcanum entry table
#[derive(Debug, DekuRead, DekuWrite)]
#[deku(endian = "little")]
struct ArcanumFileEntry {
    /// Length of the name including its NUL terminator
    name_len: u32,
    #[deku(count = "name_len")]
    name_bytes: Vec<u8>,
    /// Ignored on read. Shipped archives carry a distinct pointer-like value
    /// per entry (evidently the original tool's in-memory state); community
    /// tools write 0.
    unknown: u32,
    flags: u32,
    real_size: u32,
    packed_size: u32,
    offset: u32,
}

/// Arcanum archive handler
#[derive(Debug)]
pub struct ArcanumArchive {
    files: Vec<FileEntry>,
    /// Raw archive data for reading file content
    data: Vec<u8>,
    /// Footer GUID, preserved across resaves. Zeroed for new archives: the
    /// game is not known to read it, and a fixed value keeps output
    /// deterministic (the original tool wrote a random GUID).
    guid: [u8; 16],
}

/// Check for the Arcanum magic. Neither Fallout format carries a signature
/// at this position, so the magic decides; full footer validation is
/// `from_bytes`'s job, keeping a corrupt Arcanum trailer routed here for an
/// accurate error instead of falling through to a misleading DAT2 one.
pub fn is_arcanum_format(data: &[u8]) -> bool {
    data.len() >= FOOTER_SIZE + 4 && data[data.len() - 12..data.len() - 8] == MAGIC
}

impl ArcanumArchive {
    /// Create a new empty Arcanum archive
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            data: Vec::new(),
            guid: [0; 16],
        }
    }

    /// Parse an existing Arcanum archive from raw bytes
    pub fn from_bytes(data: Vec<u8>) -> Result<Self> {
        // Footer plus the entry table's count field is the smallest valid file
        if data.len() < FOOTER_SIZE + 4 {
            bail!("Arcanum DAT file too small");
        }

        let (_, footer) = ArcanumFooter::from_bytes((&data[data.len() - FOOTER_SIZE..], 0))
            .map_err(|e| anyhow::anyhow!("Failed to parse Arcanum footer: {e}"))?;
        if footer.magic != MAGIC {
            bail!("Not an Arcanum DAT archive: missing 1TAD magic");
        }

        // table_from_end is untrusted archive input: the table must start
        // inside the file and hold at least its 4-byte entry count.
        let table_from_end = footer.table_from_end as usize;
        if table_from_end < FOOTER_SIZE + 4 || table_from_end > data.len() {
            bail!("Invalid Arcanum footer: entry table offset out of range");
        }
        let table_start = data.len() - table_from_end;
        let table = &data[table_start..data.len() - FOOTER_SIZE];

        let mut cursor = Cursor::new(table);
        let entry_count = cursor
            .read_u32::<LittleEndian>()
            .context("Failed to read Arcanum entry count")?;

        // No preallocation from entry_count: it is untrusted input
        let mut files = Vec::new();
        let mut current_offset = 4usize;

        for i in 0..entry_count {
            let remaining = &table[current_offset..];
            let ((remaining_slice, _bit_offset), entry) =
                ArcanumFileEntry::from_bytes((remaining, 0))
                    .map_err(|e| anyhow::anyhow!("Failed to parse Arcanum entry {i}: {e}"))?;
            current_offset += remaining.len() - remaining_slice.len();

            // Directory entries carry no data; directories are recreated
            // from file paths on extraction.
            if entry.flags & FLAG_DIR != 0 {
                continue;
            }

            let name = utils::decode_filename(&entry.name_bytes)
                .with_context(|| format!("Failed to decode filename for Arcanum entry {i}"))?;

            files.push(FileEntry {
                name,
                offset: entry.offset as u64,
                size: entry.real_size,
                packed_size: entry.packed_size,
                compressed: entry.flags & FLAG_ZLIB != 0,
                data: None,
            });
        }

        Ok(Self {
            files,
            data,
            guid: footer.guid,
        })
    }

    /// List files in the archive (all or filtered by patterns)
    pub fn list(&self, files: &[String]) -> Result<()> {
        let all_files: Vec<&FileEntry> = self.files.iter().collect();
        common::list_files_filtered(&all_files, files)
    }

    /// Extract files from the archive using parallel processing
    pub fn extract(&self, output_dir: &Path, files: &[String], mode: ExtractionMode) -> Result<()> {
        let files_to_extract = common::filter_files_by_patterns(&self.files, files);
        common::extract_zlib_archive_parallel(&self.data, &files_to_extract, output_dir, mode)
    }

    /// Read file data from the archive's own data buffer
    fn read_file_data(&self, file: &FileEntry) -> Result<Vec<u8>> {
        utils::read_file_slice(&self.data, file)
    }

    /// Add files to the archive (directories processed recursively, parallel)
    pub fn add_file(
        &mut self,
        file_path: &Path,
        compression: CompressionLevel,
        target_dir: Option<&str>,
        source_root: Option<&Path>,
    ) -> Result<()> {
        common::add_files_zlib(
            &mut self.files,
            file_path,
            compression,
            target_dir,
            source_root,
        )
    }

    /// Delete a file from the archive by name
    pub fn delete_file(&mut self, file_name: &str) -> Result<()> {
        common::delete_file_from_list(&mut self.files, file_name)
    }

    /// Save the archive to an Arcanum DAT file.
    ///
    /// Layout: file data, table marker, entry table, 28-byte footer.
    pub fn save(&self, path: &Path) -> Result<()> {
        // Offsets are u32 and the table marker adds 4 bytes past the data.
        // Entries keep data.len() == packed_size, bounding the accumulation.
        let total_payload: u64 = self.files.iter().map(|f| f.packed_size as u64).sum();
        if total_payload > u32::MAX as u64 - 4 {
            bail!("Arcanum archive would exceed the format's 4 GiB offset limit");
        }

        // The table stores explicit directory entries interleaved with files
        // in one flat case-insensitive path order, matching the original
        // tool's layout. Directories are synthesized from file paths, so
        // empty directories of an opened archive are not preserved.
        let mut dir_names: Vec<String> = Vec::new();
        let mut seen_dirs = std::collections::HashSet::new();
        for file in &self.files {
            let mut pos = 0;
            while let Some(sep) = file.name[pos..].find('\\') {
                pos += sep;
                let dir = &file.name[..pos];
                if seen_dirs.insert(dir.to_lowercase()) {
                    dir_names.push(dir.to_string());
                }
                pos += 1;
            }
        }

        // (name, index into self.files; None marks a directory)
        let mut table: Vec<(&str, Option<usize>)> =
            dir_names.iter().map(|d| (d.as_str(), None)).collect();
        table.extend(
            self.files
                .iter()
                .enumerate()
                .map(|(i, f)| (f.name.as_str(), Some(i))),
        );
        table.sort_by_key(|(name, _)| name.to_lowercase());

        utils::write_atomically(path, |out| {
            // Step 1: file data, written in table order like the original tool
            let mut file_offsets = vec![0u32; self.files.len()];
            let mut current_offset = 0u32;
            for (_, index) in &table {
                if let Some(i) = index {
                    let file = &self.files[*i];

                    let owned;
                    let data: &[u8] = match file.data {
                        Some(ref file_data) => file_data,
                        None => {
                            owned = self.read_file_data(file)?;
                            &owned
                        }
                    };

                    file_offsets[*i] = current_offset;
                    out.write_all(data)?;
                    current_offset += data.len() as u32;
                }
            }

            // Step 2: table marker - the entry table's absolute offset,
            // which sits just past this u32
            out.write_u32::<LittleEndian>(current_offset + 4)?;

            // Step 3: entry table, tracking its size for the footer
            out.write_u32::<LittleEndian>(table.len() as u32)?;
            let mut table_size: u64 = 4;
            let mut names_len: u64 = 0;

            for (name, index) in &table {
                let mut name_bytes = name.as_bytes().to_vec();
                name_bytes.push(0);
                // unknown is 0: shipped archives carry junk there (see the
                // field's doc) and no reader is known to use it
                let entry = match index {
                    Some(i) => {
                        let f = &self.files[*i];
                        ArcanumFileEntry {
                            name_len: name.len() as u32 + 1,
                            name_bytes,
                            unknown: 0,
                            flags: if f.compressed { FLAG_ZLIB } else { FLAG_RAW },
                            real_size: f.size,
                            packed_size: f.packed_size,
                            offset: file_offsets[*i],
                        }
                    }
                    None => ArcanumFileEntry {
                        name_len: name.len() as u32 + 1,
                        name_bytes,
                        unknown: 0,
                        flags: FLAG_DIR,
                        real_size: 0,
                        packed_size: 0,
                        offset: 0,
                    },
                };

                let entry_bytes = entry.to_bytes()?;
                out.write_all(&entry_bytes)?;
                table_size += entry_bytes.len() as u64;
                names_len += name.len() as u64 + 1;
            }

            // Step 4: footer
            let footer = ArcanumFooter {
                guid: self.guid,
                magic: MAGIC,
                filename_total_bytes: u32::try_from(names_len)
                    .context("Arcanum archive filenames exceed the format's u32 limit")?,
                table_from_end: u32::try_from(table_size + FOOTER_SIZE as u64)
                    .context("Arcanum entry table would exceed the format's u32 limit")?,
            };
            out.write_all(&footer.to_bytes()?)?;

            Ok(())
        })
        .context("Failed to write Arcanum DAT file")?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::io::Write;

    /// One entry-table record: name_len (incl. NUL), name, NUL, unknown,
    /// flags, real_size, packed_size, offset - all u32 little-endian.
    fn entry_record(
        name: &str,
        flags: u32,
        real_size: u32,
        packed_size: u32,
        offset: u32,
    ) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(name.len() as u32 + 1).to_le_bytes());
        v.extend_from_slice(name.as_bytes());
        v.push(0);
        v.extend_from_slice(&0u32.to_le_bytes()); // unknown field
        v.extend_from_slice(&flags.to_le_bytes());
        v.extend_from_slice(&real_size.to_le_bytes());
        v.extend_from_slice(&packed_size.to_le_bytes());
        v.extend_from_slice(&offset.to_le_bytes());
        v
    }

    /// Build a complete archive from (path, flags, stored bytes, real size).
    /// Directory entries pass empty stored bytes and real size 0.
    fn build_archive(entries: &[(&str, u32, Vec<u8>, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut records = Vec::new();
        let mut names_len = 0u32;

        for (name, flags, stored, real_size) in entries {
            let offset = if flags & FLAG_DIR != 0 {
                0
            } else {
                out.len() as u32
            };
            out.extend_from_slice(stored);
            records.push(entry_record(
                name,
                *flags,
                *real_size,
                stored.len() as u32,
                offset,
            ));
            names_len += name.len() as u32 + 1;
        }

        // Table marker: absolute offset of the entry table (just past this u32)
        out.extend_from_slice(&(out.len() as u32 + 4).to_le_bytes());

        let table_start = out.len();
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for r in &records {
            out.extend_from_slice(r);
        }
        let table_len = out.len() - table_start;

        out.extend_from_slice(&[0u8; 16]); // GUID
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&names_len.to_le_bytes());
        out.extend_from_slice(&((table_len + FOOTER_SIZE) as u32).to_le_bytes());
        out
    }

    fn zlib_compress(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn detects_arcanum_magic() {
        let archive = build_archive(&[("A.TXT", FLAG_RAW, b"hi".to_vec(), 2)]);
        assert!(is_arcanum_format(&archive));
    }

    #[test]
    fn rejects_missing_magic_and_short_data() {
        let mut archive = build_archive(&[("A.TXT", FLAG_RAW, b"hi".to_vec(), 2)]);
        let magic_pos = archive.len() - 12;
        archive[magic_pos] = b'X';
        assert!(!is_arcanum_format(&archive));
        assert!(!is_arcanum_format(&[0u8; 27]));
    }

    #[test]
    fn detects_magic_even_with_corrupt_footer() {
        // A corrupt footer must still route to the Arcanum parser so the user
        // gets an Arcanum error, not a misleading DAT2 fallback one.
        let mut archive = build_archive(&[("A.TXT", FLAG_RAW, b"hi".to_vec(), 2)]);
        let len = archive.len();
        archive[len - 4..].copy_from_slice(&(len as u32 + 1).to_le_bytes());
        assert!(is_arcanum_format(&archive));
        let err = ArcanumArchive::from_bytes(archive).unwrap_err();
        assert!(
            err.to_string().contains("entry table offset out of range"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parses_files_and_skips_directory_entries() {
        let archive = build_archive(&[
            ("ART", FLAG_DIR, Vec::new(), 0),
            ("ART\\A.TXT", FLAG_RAW, b"hello".to_vec(), 5),
            ("ART\\B.BIN", FLAG_ZLIB, zlib_compress(b"world!"), 6),
        ]);

        let parsed = ArcanumArchive::from_bytes(archive).unwrap();
        assert_eq!(parsed.files.len(), 2);

        assert_eq!(parsed.files[0].name, "ART\\A.TXT");
        assert!(!parsed.files[0].compressed);
        assert_eq!(parsed.files[0].size, 5);
        assert_eq!(parsed.files[0].packed_size, 5);
        assert_eq!(parsed.files[0].offset, 0);

        assert_eq!(parsed.files[1].name, "ART\\B.BIN");
        assert!(parsed.files[1].compressed);
        assert_eq!(parsed.files[1].size, 6);
        assert_eq!(parsed.files[1].offset, 5);
    }

    #[test]
    fn extracts_raw_and_zlib_entries() {
        let archive = build_archive(&[
            ("DIR", FLAG_DIR, Vec::new(), 0),
            ("DIR\\PLAIN.TXT", FLAG_RAW, b"plain data".to_vec(), 10),
            (
                "DIR\\PACKED.TXT",
                FLAG_ZLIB,
                zlib_compress(b"packed data"),
                11,
            ),
        ]);
        let parsed = ArcanumArchive::from_bytes(archive).unwrap();

        let out_dir = std::env::temp_dir().join(format!("dat3_arc_x_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out_dir);
        parsed
            .extract(&out_dir, &[], ExtractionMode::PreserveStructure)
            .unwrap();

        let plain = std::fs::read(out_dir.join("DIR").join("PLAIN.TXT")).unwrap();
        let packed = std::fs::read(out_dir.join("DIR").join("PACKED.TXT")).unwrap();
        std::fs::remove_dir_all(&out_dir).unwrap();
        assert_eq!(plain, b"plain data");
        assert_eq!(packed, b"packed data");
    }

    #[test]
    fn errors_on_bad_magic() {
        let mut archive = build_archive(&[("A.TXT", FLAG_RAW, b"hi".to_vec(), 2)]);
        let magic_pos = archive.len() - 12;
        archive[magic_pos] = b'X';
        assert!(ArcanumArchive::from_bytes(archive).is_err());
    }

    #[test]
    fn errors_on_hostile_table_offset() {
        let mut archive = build_archive(&[("A.TXT", FLAG_RAW, b"hi".to_vec(), 2)]);
        let len = archive.len();
        archive[len - 4..].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(ArcanumArchive::from_bytes(archive).is_err());
    }

    #[test]
    fn dat_archive_open_autodetects_arcanum_and_supports_writes() {
        let archive = build_archive(&[("A.TXT", FLAG_RAW, b"hi".to_vec(), 2)]);
        let path = std::env::temp_dir().join(format!("dat3_arc_open_{}.dat", std::process::id()));
        std::fs::write(&path, &archive).unwrap();
        let opened = crate::common::DatArchive::open(&path);

        let mut opened = opened.unwrap();
        assert!(matches!(opened, crate::common::DatArchive::Arcanum(_)));
        opened.delete_file("A.TXT").unwrap();
        opened.save(&path).unwrap();

        let reopened = crate::common::DatArchive::open(&path);
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(
            reopened.unwrap(),
            crate::common::DatArchive::Arcanum(_)
        ));
    }

    #[test]
    fn save_writes_reference_layout() {
        // Deliberately unsorted, with paths needing "A" and "b" dir synthesis
        let mut archive = ArcanumArchive::new();
        let mut f1 = FileEntry::with_data("b\\y.txt".to_string(), b"yy".to_vec(), false);
        f1.size = 2;
        let mut f2 = FileEntry::with_data("A\\x.txt".to_string(), b"xxx".to_vec(), false);
        f2.size = 3;
        archive.files.push(f1);
        archive.files.push(f2);

        let path = std::env::temp_dir().join(format!("dat3_arc_layout_{}.dat", std::process::id()));
        archive.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        // Data section: payloads in table (sorted) order, then the marker
        // holding the entry table's absolute offset
        assert_eq!(&bytes[..5], b"xxxyy");
        assert_eq!(&bytes[5..9], &9u32.to_le_bytes());
        assert_eq!(&bytes[9..13], &4u32.to_le_bytes()); // 2 dirs + 2 files

        // Records in flat case-insensitive path order, dirs interleaved
        let mut off = 13usize;
        let mut seen = Vec::new();
        for _ in 0..4 {
            let name_len = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
            let name = std::str::from_utf8(&bytes[off + 4..off + 4 + name_len - 1]).unwrap();
            assert_eq!(bytes[off + 4 + name_len - 1], 0, "name not NUL-terminated");
            off += 4 + name_len;
            let words: Vec<u32> = (0..5)
                .map(|i| {
                    u32::from_le_bytes(bytes[off + 4 * i..off + 4 * i + 4].try_into().unwrap())
                })
                .collect();
            off += 20;
            seen.push((name.to_string(), words));
        }
        assert_eq!(
            seen.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            vec!["A", "A\\x.txt", "b", "b\\y.txt"]
        );
        // Dir entries: unknown 0, FLAG_DIR, zero sizes and offset
        assert_eq!(seen[0].1, vec![0, FLAG_DIR, 0, 0, 0]);
        assert_eq!(seen[2].1, vec![0, FLAG_DIR, 0, 0, 0]);
        // File entries: unknown 0, FLAG_RAW, sizes, data offset in table order
        assert_eq!(seen[1].1, vec![0, FLAG_RAW, 3, 3, 0]);
        assert_eq!(seen[3].1, vec![0, FLAG_RAW, 2, 2, 3]);

        // Footer: names_len counts every entry's name incl. NUL
        let names_len: u32 = seen.iter().map(|(n, _)| n.len() as u32 + 1).sum();
        let flen = bytes.len();
        assert_eq!(&bytes[flen - 12..flen - 8], &MAGIC);
        assert_eq!(&bytes[flen - 8..flen - 4], &names_len.to_le_bytes());
        let table_len = flen - FOOTER_SIZE - 9;
        assert_eq!(
            &bytes[flen - 4..],
            &((table_len + FOOTER_SIZE) as u32).to_le_bytes()
        );
    }

    #[test]
    fn resave_preserves_guid() {
        let mut archive = build_archive(&[("A.TXT", FLAG_RAW, b"hi".to_vec(), 2)]);
        let guid_pos = archive.len() - FOOTER_SIZE;
        let guid: Vec<u8> = (1..=16).collect();
        archive[guid_pos..guid_pos + 16].copy_from_slice(&guid);

        let parsed = ArcanumArchive::from_bytes(archive).unwrap();
        let path = std::env::temp_dir().join(format!("dat3_arc_guid_{}.dat", std::process::id()));
        parsed.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(&bytes[bytes.len() - FOOTER_SIZE..bytes.len() - 12], &guid);
    }

    #[test]
    fn add_file_compresses_and_round_trips() {
        let src_dir = std::env::temp_dir().join(format!("dat3_arc_add_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&src_dir);
        std::fs::create_dir_all(src_dir.join("sub")).unwrap();
        let payload = b"compress me please, compress me please".repeat(10);
        std::fs::write(src_dir.join("sub").join("data.txt"), &payload).unwrap();

        let mut archive = ArcanumArchive::new();
        archive
            .add_file(
                &src_dir,
                crate::common::CompressionLevel::new(9).unwrap(),
                None,
                None,
            )
            .unwrap();
        let path = std::env::temp_dir().join(format!("dat3_arc_addrt_{}.dat", std::process::id()));
        archive.save(&path).unwrap();

        let reparsed = ArcanumArchive::from_bytes(std::fs::read(&path).unwrap()).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(reparsed.files.len(), 1);
        let entry = &reparsed.files[0];
        assert!(
            entry.name.ends_with("sub\\data.txt"),
            "name: {}",
            entry.name
        );
        assert!(
            entry.compressed,
            "compressible payload should be compressed"
        );
        assert_eq!(entry.size as usize, payload.len());

        let out_dir = std::env::temp_dir().join(format!("dat3_arc_addx_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out_dir);
        reparsed
            .extract(&out_dir, &[], ExtractionMode::Flat)
            .unwrap();
        let extracted = std::fs::read(out_dir.join("data.txt")).unwrap();
        std::fs::remove_dir_all(&out_dir).unwrap();
        std::fs::remove_dir_all(&src_dir).unwrap();
        assert_eq!(extracted, payload);
    }

    #[test]
    fn save_errors_when_payload_exceeds_u32_offsets() {
        let huge_entry = |name: &str| FileEntry {
            name: name.to_string(),
            offset: 0,
            size: u32::MAX,
            packed_size: u32::MAX,
            compressed: false,
            data: Some(Vec::new()),
        };
        let archive = ArcanumArchive {
            files: vec![huge_entry("A.TXT"), huge_entry("B.TXT")],
            data: Vec::new(),
            guid: [0; 16],
        };
        let target = std::env::temp_dir().join("dat3_arc_overflow_test.dat");
        assert!(archive.save(&target).is_err());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn save_then_parse_round_trips(
            payloads in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..512), 1..8)
        ) {
            let mut archive = ArcanumArchive::new();
            for (i, data) in payloads.iter().enumerate() {
                // Half the entries live in a subdirectory to exercise dir synthesis
                let name = if i % 2 == 0 { format!("F{i}.BIN") } else { format!("SUB\\F{i}.BIN") };
                let mut entry = FileEntry::with_data(name, data.clone(), false);
                entry.size = data.len() as u32;
                archive.files.push(entry);
            }

            let target = std::env::temp_dir()
                .join(format!("dat3_prop_arc_{}.dat", std::process::id()));
            archive.save(&target).unwrap();
            let bytes = std::fs::read(&target).unwrap();
            std::fs::remove_file(&target).unwrap();
            prop_assert!(is_arcanum_format(&bytes));

            let reparsed = ArcanumArchive::from_bytes(bytes).unwrap();
            prop_assert_eq!(reparsed.files.len(), payloads.len());
            for (i, data) in payloads.iter().enumerate() {
                let name = if i % 2 == 0 { format!("F{i}.BIN") } else { format!("SUB\\F{i}.BIN") };
                let entry = reparsed.files.iter().find(|f| f.name == name).unwrap();
                prop_assert_eq!(utils::read_file_slice(&reparsed.data, entry).unwrap(), data.clone());
            }
        }

        #[test]
        fn from_bytes_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
            let _ = ArcanumArchive::from_bytes(bytes);
        }
    }
}
