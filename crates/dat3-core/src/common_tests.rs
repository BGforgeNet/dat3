/*!
Unit tests for common module utilities.

Tests cover path handling, pattern matching, filename validation,
compression levels, filtering, and path traversal protection.
*/

#[cfg(test)]
mod tests {
    use crate::common::*;
    use std::path::Path;

    // ── CompressionLevel ───────────────────────────────────────────

    mod compression_level {
        use super::*;

        #[test]
        fn valid_levels_0_through_9() {
            for level in 0..=9 {
                let result = CompressionLevel::new(level);
                assert!(result.is_ok(), "Level {} should be valid", level);
                assert_eq!(result.unwrap().level(), level);
            }
        }

        #[test]
        fn rejects_level_10() {
            assert!(CompressionLevel::new(10).is_err());
        }

        #[test]
        fn rejects_level_255() {
            assert!(CompressionLevel::new(255).is_err());
        }
    }

    // ── decompress_zlib ────────────────────────────────────────────

    mod decompress_zlib {
        use super::*;
        use std::io::Write;

        fn compressed(data: &[u8]) -> Vec<u8> {
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
            enc.write_all(data).unwrap();
            enc.finish().unwrap()
        }

        #[test]
        fn decompresses_to_the_declared_size() {
            assert_eq!(decompress_zlib(&compressed(b"ABC"), 3).unwrap(), b"ABC");
        }

        #[test]
        fn stops_at_the_declared_size_when_the_stream_is_longer() {
            // A small entry declaring a tiny size must not expand to its full
            // stream: that is how a crafted archive fills a disk.
            let err = decompress_zlib(&compressed(&[0u8; 1 << 20]), 10).unwrap_err();
            assert!(
                err.to_string()
                    .contains("exceeds the declared size of 10 bytes"),
                "unexpected error: {err}"
            );
        }

        #[test]
        fn errors_when_the_stream_is_shorter_than_declared() {
            let err = decompress_zlib(&compressed(b"ABC"), 4).unwrap_err();
            assert!(
                err.to_string()
                    .contains("3 bytes, but the archive declares 4"),
                "unexpected error: {err}"
            );
        }

        #[test]
        fn does_not_trust_hostile_expected_size() {
            // expected_size comes from archive metadata; a crafted value must
            // not trigger a giant (or panicking) upfront allocation.
            assert!(decompress_zlib(&compressed(b"ABC"), usize::MAX).is_err());
        }
    }

    // ── deku_parse_error ───────────────────────────────────────────

    mod deku_parse_error {
        use super::*;
        use deku::DekuError;
        use deku::error::NeedSize;

        #[test]
        fn reports_a_short_read_in_bytes() {
            let err = deku_parse_error(
                "Failed to parse entry 3",
                DekuError::Incomplete(NeedSize::new(776)),
            );
            let text = err.to_string();
            assert_eq!(
                text,
                "Failed to parse entry 3: archive data ends early (97 more bytes needed)"
            );
        }

        #[test]
        fn reports_a_failed_field_check_as_an_invalid_value() {
            let err = deku_parse_error(
                "Failed to parse entry 3",
                DekuError::Assertion("Dat2FileEntry.filename_size".into()),
            );
            assert_eq!(
                err.to_string(),
                "Failed to parse entry 3: invalid value (Dat2FileEntry.filename_size)"
            );
        }

        #[test]
        fn keeps_other_errors_with_their_context() {
            let err = deku_parse_error("Failed to parse entry 3", DekuError::Parse("bad".into()));
            assert_eq!(err.to_string(), "Failed to parse entry 3: Parse error: bad");
        }
    }

    // ── resolve_output_path ────────────────────────────────────────

    mod resolve_output_path {
        use super::*;
        use std::path::PathBuf;

        #[test]
        fn flat_mode_joins_bare_filename() {
            let p = utils::resolve_output_path(
                Path::new("out"),
                "ART\\CRITTERS\\FILE.FRM",
                ExtractionMode::Flat,
            );
            assert_eq!(p, PathBuf::from("out").join("FILE.FRM"));
        }

        #[test]
        fn preserve_mode_joins_full_system_path() {
            let p = utils::resolve_output_path(
                Path::new("out"),
                "ART\\CRITTERS\\FILE.FRM",
                ExtractionMode::PreserveStructure,
            );
            #[cfg(not(windows))]
            assert_eq!(p, PathBuf::from("out/ART/CRITTERS/FILE.FRM"));
            #[cfg(windows)]
            assert_eq!(p, PathBuf::from("out\\ART\\CRITTERS\\FILE.FRM"));
        }
    }

    // ── case handling ──────────────────────────────────────────────

    mod case_handling {
        use super::*;

        #[test]
        fn groups_only_names_that_differ_in_case() {
            let names = ["A.TXT", "b.txt", "a.txt", "B.TXT", "c.txt", "a.txt"];
            assert_eq!(
                case_only_duplicates(names),
                [vec!["A.TXT", "a.txt"], vec!["B.TXT", "b.txt"]]
            );
        }

        #[test]
        fn shows_names_in_lowercase_unless_a_case_only_twin_exists() {
            let view = NameView::new(CaseMode::Insensitive, ["ART\\HERO.FRM", "A.TXT", "a.txt"]);
            assert_eq!(view.shown("ART\\HERO.FRM"), "art\\hero.frm");
            assert_eq!(view.shown("A.TXT"), "A.TXT");
            assert_eq!(view.shown("a.txt"), "a.txt");
        }

        #[test]
        fn shows_stored_names_when_case_sensitive() {
            let view = NameView::new(CaseMode::Sensitive, ["ART\\HERO.FRM"]);
            assert_eq!(view.shown("ART\\HERO.FRM"), "ART\\HERO.FRM");
        }

        #[test]
        fn json_listing_uses_the_shown_names() {
            let entry = FileEntry {
                name: "ART\\HERO.FRM".to_string(),
                offset: 0,
                size: 1,
                packed_size: 1,
                compressed: false,
                data: None,
            };
            let files = [entry];
            let view = NameView::new(CaseMode::Insensitive, ["ART\\HERO.FRM"]);
            assert!(
                utils::format_file_listing_json(&files, &view)
                    .contains("\"name\": \"art/hero.frm\""),
                "{}",
                utils::format_file_listing_json(&files, &view)
            );
        }

        /// A plain name ignoring case selects every stored spelling of it.
        #[test]
        fn delete_by_plain_name_selects_every_case_only_twin() {
            let names = ["A.TXT", "a.txt", "OTHER.TXT"];
            assert_eq!(
                resolve_delete_targets(&names, &["a.txt".to_string()], CaseMode::Insensitive)
                    .unwrap(),
                ["A.TXT", "a.txt"]
            );
            assert_eq!(
                resolve_delete_targets(&names, &["a.txt".to_string()], CaseMode::Sensitive)
                    .unwrap(),
                ["a.txt"]
            );
        }

        /// Matching, deleting and showing fold case the same way, beyond ASCII
        /// too, so a name matches exactly when it would be shown alike.
        #[test]
        fn non_ascii_names_fold_alike_everywhere() {
            let stored = "DATA\\\u{c9}T\u{c9}.TXT";
            let typed = "data\\\u{e9}t\u{e9}.txt";
            let view = NameView::new(CaseMode::Insensitive, [stored]);
            assert_eq!(view.shown(stored), typed);

            let matches = |pattern: &str| {
                utils::NamePattern::new(pattern)
                    .unwrap()
                    .matches(stored, CaseMode::Insensitive)
            };
            assert!(matches(typed), "substring");
            assert!(matches("data/\u{e9}t*.txt"), "glob, lowercase pattern");
            assert!(
                utils::NamePattern::new("DATA/\u{c9}T*.TXT")
                    .unwrap()
                    .matches("data\\\u{e9}t\u{e9}.txt", CaseMode::Insensitive),
                "glob, uppercase pattern"
            );
            assert_eq!(
                resolve_delete_targets(&[stored], &[typed.to_string()], CaseMode::Insensitive)
                    .unwrap(),
                [stored]
            );
        }
    }

    // ── progress_line ──────────────────────────────────────────────

    mod progress_line {
        use super::*;
        use std::time::Duration;

        #[test]
        fn omits_the_rate_when_no_time_has_passed() {
            assert_eq!(
                progress_line(2, 2, Duration::ZERO),
                "Progress: 2/2 files extracted"
            );
        }

        #[test]
        fn reports_the_rate_in_files_per_second() {
            assert_eq!(
                progress_line(3, 10, Duration::from_millis(1500)),
                "Progress: 3/10 files extracted (2.0 files/sec)"
            );
        }
    }

    // ── flat extraction ────────────────────────────────────────────

    mod flat_extraction {
        use super::*;

        fn entry(name: &str, data: &[u8]) -> FileEntry {
            let mut entry = FileEntry::with_data(name.to_string(), data.to_vec(), false);
            entry.size = data.len() as u32;
            entry
        }

        #[test]
        fn keeps_only_the_last_entry_for_each_file_name() {
            let entries = [
                entry("A\\SAME.TXT", b"first"),
                entry("OTHER.TXT", b"other"),
                entry("B\\SAME.TXT", b"second"),
            ];
            let refs: Vec<&FileEntry> = entries.iter().collect();
            let (kept, replaced) = utils::last_entry_per_flat_name(&refs);
            let names: Vec<&str> = kept.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, ["OTHER.TXT", "B\\SAME.TXT"]);
            let replaced: Vec<&str> = replaced.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(replaced, ["A\\SAME.TXT"]);
        }

        /// Names differing only in case land on one file on Windows and macOS.
        #[test]
        fn treats_names_differing_only_in_case_as_the_same_file() {
            let entries = [entry("A\\X.TXT", b"first"), entry("B\\x.txt", b"second")];
            let refs: Vec<&FileEntry> = entries.iter().collect();
            let (kept, replaced) = utils::last_entry_per_flat_name(&refs);
            let kept: Vec<&str> = kept.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(kept, ["B\\x.txt"]);
            assert_eq!(replaced.len(), 1);
        }

        /// An unsafe name must stop the extraction before any file is written,
        /// not after parallel workers have already written the others.
        #[test]
        fn an_unsafe_entry_name_fails_before_anything_is_written() {
            let mut entries: Vec<FileEntry> = (0..200)
                .map(|i| entry(&format!("DATA\\F{i}.TXT"), b"data"))
                .collect();
            entries.push(entry("AUX\\B.TXT", b"data"));
            let refs: Vec<&FileEntry> = entries.iter().collect();
            let out = crate::test_support::ScratchPath::dir("unsafe_before_write");

            let result = extract_archive_parallel(
                &[],
                &refs,
                &out,
                ExtractionMode::PreserveStructure,
                &NameView::new(CaseMode::Sensitive, []),
                |d, _| Ok(d.to_vec()),
            );

            assert!(result.is_err());
            assert_eq!(std::fs::read_dir(&out).unwrap().count(), 0);
        }

        /// Entries sharing a file name used to race to one output path from
        /// parallel workers, so which one survived varied from run to run.
        #[test]
        fn writes_the_last_entry_when_names_collide() {
            let entries = [
                entry("A\\SAME.TXT", b"first"),
                entry("B\\SAME.TXT", b"second"),
            ];
            let refs: Vec<&FileEntry> = entries.iter().collect();
            let out = crate::test_support::ScratchPath::dir("flat_collision");
            extract_archive_parallel(
                &[],
                &refs,
                &out,
                ExtractionMode::Flat,
                &NameView::new(CaseMode::Sensitive, []),
                |d, _| Ok(d.to_vec()),
            )
            .unwrap();
            assert_eq!(std::fs::read(out.join("SAME.TXT")).unwrap(), b"second");
        }
    }

    // ── read_file_slice ────────────────────────────────────────────

    mod read_file_slice {
        use super::*;

        fn stored_entry(offset: u64, packed_size: u32) -> FileEntry {
            FileEntry {
                name: "A\\B.TXT".to_string(),
                offset,
                size: packed_size,
                packed_size,
                compressed: false,
                data: None,
            }
        }

        #[test]
        fn returns_in_memory_data_when_present() {
            let mut entry = stored_entry(0, 3);
            entry.data = Some(vec![1, 2, 3]);
            let out = utils::read_file_slice(&[], &entry).unwrap();
            assert_eq!(out, vec![1, 2, 3]);
        }

        #[test]
        fn slices_at_offset_and_packed_size() {
            let archive = [0u8, 10, 20, 30, 40];
            let entry = stored_entry(1, 3);
            let out = utils::read_file_slice(&archive, &entry).unwrap();
            assert_eq!(out, [10, 20, 30]);
        }

        #[test]
        fn errors_when_range_exceeds_archive() {
            let archive = [0u8; 4];
            assert!(utils::read_file_slice(&archive, &stored_entry(2, 3)).is_err());
        }

        #[test]
        fn errors_on_offset_overflow_instead_of_panicking() {
            let archive = [0u8; 4];
            assert!(utils::read_file_slice(&archive, &stored_entry(u64::MAX, 1)).is_err());
        }
    }

    // ── write_atomically ───────────────────────────────────────────

    mod write_atomically {
        use super::*;
        use std::fs;
        use std::io::Write;

        fn scratch_dir(tag: &str) -> crate::test_support::ScratchPath {
            crate::test_support::ScratchPath::dir(tag)
        }

        #[test]
        fn writes_bytes_to_target() {
            let dir = scratch_dir("write");
            let target = dir.join("out.dat");
            utils::write_atomically(&target, |w| Ok(w.write_all(b"payload")?)).unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"payload");
        }

        #[test]
        fn replaces_existing_file() {
            let dir = scratch_dir("replace");
            let target = dir.join("out.dat");
            fs::write(&target, b"old").unwrap();
            utils::write_atomically(&target, |w| Ok(w.write_all(b"new")?)).unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"new");
        }

        #[test]
        fn leaves_no_temp_residue() {
            let dir = scratch_dir("residue");
            let target = dir.join("out.dat");
            utils::write_atomically(&target, |w| Ok(w.write_all(b"payload")?)).unwrap();
            let names: Vec<_> = fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, vec![std::ffi::OsString::from("out.dat")]);
        }

        #[test]
        fn errors_when_parent_directory_missing() {
            let target = std::env::temp_dir()
                .join(format!("dat3_wa_missing_{}", std::process::id()))
                .join("nope")
                .join("out.dat");
            assert!(utils::write_atomically(&target, |w| Ok(w.write_all(b"payload")?)).is_err());
        }

        #[test]
        fn removes_temp_and_preserves_target_when_write_fails() {
            let dir = scratch_dir("failpath");
            let target = dir.join("out.dat");
            fs::write(&target, b"old").unwrap();
            let result = utils::write_atomically(&target, |w| {
                w.write_all(b"partial")?;
                anyhow::bail!("simulated mid-save failure")
            });
            assert!(result.is_err());
            assert_eq!(fs::read(&target).unwrap(), b"old");
            let names: Vec<_> = fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            assert_eq!(names, vec![std::ffi::OsString::from("out.dat")]);
        }

        /// Saving replaces the file, so without this a read-only or group-shared
        /// archive would come back with the process umask's default mode.
        #[cfg(unix)]
        #[test]
        fn keeps_the_permissions_of_the_archive_it_replaces() {
            use std::os::unix::fs::PermissionsExt;

            let dir = scratch_dir("perms");
            let target = dir.join("out.dat");
            fs::write(&target, b"old").unwrap();
            fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();

            utils::write_atomically(&target, |w| Ok(w.write_all(b"new")?)).unwrap();

            let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o640, "mode was {mode:o}");
        }

        /// The temp name must not grow with the archive's name: most filesystems cap
        /// a name at 255 bytes, and an archive near that limit still has to save.
        #[test]
        fn saves_an_archive_whose_name_is_near_the_filesystem_limit() {
            let dir = scratch_dir("longname");
            let target = dir.join(format!("{}.dat", "A".repeat(246)));
            utils::write_atomically(&target, |w| Ok(w.write_all(b"new")?)).unwrap();
            assert_eq!(fs::read(&target).unwrap(), b"new");
        }

        /// Two saves of one archive at once must not write into, or rename away,
        /// each other's temp file.
        #[test]
        fn does_not_touch_a_temp_file_another_save_left_in_place() {
            let dir = scratch_dir("othertemp");
            let target = dir.join("out.dat");
            let other = dir.join(".out.dat.tmp");
            fs::write(&other, b"another save in progress").unwrap();

            utils::write_atomically(&target, |w| Ok(w.write_all(b"new")?)).unwrap();

            assert_eq!(fs::read(&target).unwrap(), b"new");
            assert_eq!(fs::read(&other).unwrap(), b"another save in progress");
        }
    }

    // ── DAT1 format detection ──────────────────────────────────────

    /// Minimal DAT1 archive: header, one directory name, its content header,
    /// one file entry named `entry`, then that entry's two bytes of data.
    /// `folder_hint` and `file_hint` are the two allocation hints.
    fn dat1_bytes(folder_hint: u32, file_hint: u32, entry: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&1u32.to_be_bytes()); // directory_count
        v.extend_from_slice(&folder_hint.to_be_bytes());
        v.extend_from_slice(&0u32.to_be_bytes()); // reserved
        v.extend_from_slice(&0u32.to_be_bytes()); // timestamp
        v.push(1);
        v.push(b'.'); // the root directory name
        v.extend_from_slice(&1u32.to_be_bytes()); // file_count
        v.extend_from_slice(&file_hint.to_be_bytes());
        v.extend_from_slice(&16u32.to_be_bytes()); // fixed_metadata_size
        v.extend_from_slice(&0u32.to_be_bytes()); // timestamp
        v.push(entry.len() as u8);
        v.extend_from_slice(entry.as_bytes());
        v.extend_from_slice(&0x20u32.to_be_bytes()); // attributes: stored
        // The payload starts after this entry's remaining three u32 fields.
        let data_offset = (v.len() + 12) as u32;
        v.extend_from_slice(&data_offset.to_be_bytes());
        v.extend_from_slice(&2u32.to_be_bytes()); // size
        v.extend_from_slice(&0u32.to_be_bytes()); // packed_size
        v.extend_from_slice(b"hi");
        v
    }

    mod dat1_detection {
        use super::dat1_bytes;
        use crate::archive::DatArchive;
        use crate::common::{ExtractionMode, MissingFiles};

        /// Opens `bytes` and reports whether the format detector chose DAT1.
        /// A blob the detector rejects fails to open at all - nothing else in
        /// the fallback chain can parse it - so an error counts as "not DAT1".
        fn detects_dat1(bytes: &[u8]) -> bool {
            // Every case here produces a blob of the same length, so the name
            // needs a counter: tests in one binary run concurrently.
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("dat3_detect_{}_{seq}.dat", std::process::id()));
            std::fs::write(&path, bytes).unwrap();
            let opened = DatArchive::open(&path);
            std::fs::remove_file(&path).ok();
            matches!(opened, Ok(DatArchive::Dat1(_)))
        }

        /// The retail archives happen to carry 10 and 94 here, and those two
        /// values used to be the whole test - which rejected every other real
        /// archive, the Fallout 1 demo among them (its hint is 46).
        #[test]
        fn accepts_any_allocation_hint_at_or_above_the_directory_count() {
            for hint in [1u32, 10, 46, 94, 200] {
                assert!(
                    detects_dat1(&dat1_bytes(hint, 1, "A.TXT")),
                    "hint {hint} was not detected as DAT1"
                );
            }
        }

        /// The hint is an allocation size for the directory list, so a value
        /// below the count is not a DAT1 header - it keeps the heuristic from
        /// swallowing DAT2 archives, which carry no signature at all.
        #[test]
        fn rejects_an_allocation_hint_below_the_directory_count() {
            assert!(!detects_dat1(&dat1_bytes(0, 1, "A.TXT")));
        }

        #[test]
        fn round_trips_a_detected_archive() {
            let bytes = dat1_bytes(46, 1, "A.TXT");
            let path =
                std::env::temp_dir().join(format!("dat3_detect_rt_{}.dat", std::process::id()));
            std::fs::write(&path, &bytes).unwrap();
            let archive = DatArchive::open(&path).unwrap();
            std::fs::remove_file(&path).ok();
            let dir = std::env::temp_dir().join(format!("dat3_detect_x_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            archive
                .extract(
                    &dir,
                    ExtractionMode::PreserveStructure,
                    &crate::test_support::exact(&[], MissingFiles::Fail),
                )
                .unwrap();
            let got = std::fs::read(dir.join("A.TXT")).unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
            assert_eq!(got, b"hi");
        }
    }

    // ── normalize_path_for_display ─────────────────────────────────

    mod normalize_path_for_display {
        use super::*;

        #[test]
        fn converts_backslashes_to_forward_on_unix() {
            // On non-Windows, backslashes become forward slashes
            #[cfg(not(windows))]
            assert_eq!(
                utils::normalize_path_for_display("ART\\CRITTERS\\FILE.FRM"),
                "ART/CRITTERS/FILE.FRM"
            );
        }

        #[test]
        fn no_separators_unchanged() {
            assert_eq!(utils::normalize_path_for_display("file.txt"), "file.txt");
        }

        #[test]
        fn empty_string() {
            assert_eq!(utils::normalize_path_for_display(""), "");
        }
    }

    // ── normalize_path_for_archive ─────────────────────────────────

    mod normalize_path_for_archive {
        use super::*;

        #[test]
        fn converts_forward_to_backslashes() {
            assert_eq!(
                utils::normalize_path_for_archive("art/critters/file.frm"),
                "art\\critters\\file.frm"
            );
        }

        #[test]
        fn backslashes_unchanged() {
            assert_eq!(
                utils::normalize_path_for_archive("art\\critters\\file.frm"),
                "art\\critters\\file.frm"
            );
        }

        #[test]
        fn no_separators_unchanged() {
            assert_eq!(utils::normalize_path_for_archive("file.txt"), "file.txt");
        }
    }

    // ── normalize_user_path ────────────────────────────────────────

    mod normalize_user_path {
        use super::*;

        #[test]
        fn converts_forward_slashes_to_backslashes() {
            let result = utils::normalize_user_path("art/critters/file.frm");
            assert_eq!(result.as_ref(), "art\\critters\\file.frm");
        }

        #[test]
        fn borrows_when_no_conversion_needed() {
            let result = utils::normalize_user_path("art\\critters\\file.frm");
            // Should be Cow::Borrowed (no allocation)
            assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        }

        #[test]
        fn allocates_when_conversion_needed() {
            let result = utils::normalize_user_path("art/critters/file.frm");
            assert!(matches!(result, std::borrow::Cow::Owned(_)));
        }
    }

    // ── decode_filename ────────────────────────────────────────────

    mod decode_filename {
        use super::*;

        #[test]
        fn valid_ascii() {
            assert_eq!(
                utils::decode_filename(b"CRITTERS.LST").unwrap(),
                "CRITTERS.LST"
            );
        }

        #[test]
        fn strips_null_bytes() {
            assert_eq!(
                utils::decode_filename(b"FILE.TXT\0\0\0").unwrap(),
                "FILE.TXT"
            );
        }

        #[test]
        fn rejects_non_ascii() {
            // UTF-8 encoded e-acute: 0xC3 0xA9
            assert!(utils::decode_filename(&[0xC3, 0xA9]).is_err());
        }

        #[test]
        fn empty_input() {
            assert_eq!(utils::decode_filename(b"").unwrap(), "");
        }

        #[test]
        fn null_only_input() {
            assert_eq!(utils::decode_filename(b"\0\0").unwrap(), "");
        }
    }

    // ── validate_filename_ascii ────────────────────────────────────

    mod validate_filename_ascii {
        use super::*;

        #[test]
        fn accepts_ascii() {
            assert!(utils::validate_filename_ascii("hello.txt").is_ok());
        }

        #[test]
        fn rejects_unicode() {
            assert!(utils::validate_filename_ascii("héllo.txt").is_err());
        }

        #[test]
        fn accepts_empty() {
            assert!(utils::validate_filename_ascii("").is_ok());
        }
    }

    // ── get_filename_from_dat_path ─────────────────────────────────

    mod get_filename_from_dat_path {
        use super::*;

        #[test]
        fn extracts_from_backslash_path() {
            assert_eq!(
                utils::get_filename_from_dat_path("ART\\CRITTERS\\FILE.FRM"),
                "FILE.FRM"
            );
        }

        #[test]
        fn extracts_from_forward_slash_path() {
            assert_eq!(
                utils::get_filename_from_dat_path("art/critters/file.frm"),
                "file.frm"
            );
        }

        #[test]
        fn no_separators_returns_whole_string() {
            assert_eq!(utils::get_filename_from_dat_path("file.frm"), "file.frm");
        }
    }

    // ── get_dirname_from_dat_path ──────────────────────────────────

    mod get_dirname_from_dat_path {
        use super::*;

        #[test]
        fn extracts_directory_with_backslash() {
            assert_eq!(
                utils::get_dirname_from_dat_path("ART\\CRITTERS\\FILE.FRM"),
                "ART\\CRITTERS"
            );
        }

        #[test]
        fn no_directory_returns_dot() {
            assert_eq!(utils::get_dirname_from_dat_path("FILE.FRM"), ".");
        }
    }

    // ── matches_pattern ────────────────────────────────────────────

    mod matches_pattern {
        use super::*;

        fn matches(file_name: &str, pattern: &str) -> bool {
            utils::NamePattern::new(pattern)
                .unwrap()
                .matches(file_name, CaseMode::Sensitive)
        }

        fn matches_ignoring_case(file_name: &str, pattern: &str) -> bool {
            utils::NamePattern::new(pattern)
                .unwrap()
                .matches(file_name, CaseMode::Insensitive)
        }

        #[test]
        fn rejects_an_invalid_glob_instead_of_matching_it_as_text() {
            let err = utils::NamePattern::new("ART\\[CRIT").err().unwrap();
            let expected = format!(
                "Invalid glob pattern: {}",
                utils::normalize_path_for_display("ART\\[CRIT")
            );
            assert_eq!(err.to_string(), expected);
        }

        #[test]
        fn substring_match() {
            assert!(matches("ART\\CRITTERS\\FILE.FRM", "FILE.FRM"));
        }

        #[test]
        fn substring_no_match() {
            assert!(!matches("ART\\CRITTERS\\FILE.FRM", "MISSING.TXT"));
        }

        #[test]
        fn glob_star_matches_extension() {
            assert!(matches("ART\\CRITTERS\\FILE.FRM", "*.FRM"));
        }

        #[test]
        fn globs_and_plain_names_ignore_case_by_default() {
            assert!(matches_ignoring_case(
                "ART\\CRITTERS\\FILE.FRM",
                "art/critters/*.frm"
            ));
            assert!(matches_ignoring_case("art\\critters\\file.frm", "*.FRM"));
            assert!(matches_ignoring_case("ART\\CRITTERS\\FILE.FRM", "file.frm"));
        }

        #[test]
        fn globs_and_plain_names_respect_case_when_sensitive() {
            assert!(!matches("ART\\CRITTERS\\FILE.FRM", "art/critters/*.frm"));
            assert!(!matches("ART\\CRITTERS\\FILE.FRM", "file.frm"));
        }

        #[test]
        fn glob_star_no_match_wrong_extension() {
            assert!(!matches("ART\\CRITTERS\\FILE.FRM", "*.TXT"));
        }

        #[test]
        fn glob_question_mark() {
            assert!(matches("ART\\CRITTERS\\A.FRM", "?.FRM"));
            assert!(!matches("ART\\CRITTERS\\AB.FRM", "?.FRM"));
        }

        #[test]
        fn glob_with_path_prefix() {
            assert!(matches("ART\\CRITTERS\\FILE.FRM", "ART/CRITTERS/*.FRM"));
        }

        #[test]
        fn glob_path_no_match_wrong_dir() {
            assert!(!matches("ART\\CRITTERS\\FILE.FRM", "SOUND/*.FRM"));
        }

        #[test]
        fn character_range() {
            assert!(matches("ART\\CRITTERS\\FILE1.FRM", "[A-Z]*.FRM"));
        }
    }

    // ── contains_glob_metacharacters ───────────────────────────────

    mod contains_glob_metacharacters {
        use super::*;

        #[test]
        fn detects_star() {
            assert!(utils::contains_glob_metacharacters("*.txt"));
        }

        #[test]
        fn detects_question() {
            assert!(utils::contains_glob_metacharacters("file?.txt"));
        }

        #[test]
        fn detects_bracket() {
            assert!(utils::contains_glob_metacharacters("[abc].txt"));
        }

        #[test]
        fn no_metacharacters() {
            assert!(!utils::contains_glob_metacharacters("file.txt"));
        }
    }

    // ── strip_dot_prefix_from_path ──────────────────────────────────

    mod strip_dot_prefix {
        use super::*;

        #[test]
        fn leaves_plain_path_unchanged() {
            assert_eq!(
                utils::strip_dot_prefix_from_path("patch000/file.txt"),
                "patch000/file.txt"
            );
        }

        #[test]
        fn strips_with_dot_slash_prefix() {
            assert_eq!(
                utils::strip_dot_prefix_from_path("./patch000/file.txt"),
                "patch000/file.txt"
            );
        }

        #[test]
        fn preserves_subdirectories() {
            assert_eq!(
                utils::strip_dot_prefix_from_path("./patch000/subdir/file.txt"),
                "patch000/subdir/file.txt"
            );
        }

        #[test]
        fn no_directory_returns_filename() {
            assert_eq!(utils::strip_dot_prefix_from_path("file.txt"), "file.txt");
        }

        #[test]
        fn handles_backslashes() {
            assert_eq!(
                utils::strip_dot_prefix_from_path(".\\patch000\\file.txt"),
                "patch000/file.txt"
            );
        }

        #[test]
        fn collapses_consecutive_slashes() {
            assert_eq!(
                utils::strip_dot_prefix_from_path(".\\patch000//file.txt"),
                "patch000/file.txt"
            );
        }

        #[test]
        fn strips_unix_root_prefix() {
            assert_eq!(
                utils::strip_dot_prefix_from_path("/patch000/file.txt"),
                "patch000/file.txt"
            );
        }

        #[cfg(windows)]
        #[test]
        fn strips_windows_drive_prefix() {
            assert_eq!(
                utils::strip_dot_prefix_from_path(r"C:\patch000\file.txt"),
                "patch000/file.txt"
            );
        }
    }

    // ── filter_and_track_patterns ──────────────────────────────────

    mod filter_and_track_patterns {
        use super::*;

        fn make_entry(name: &str) -> FileEntry {
            FileEntry {
                name: name.to_string(),
                offset: 0,
                size: 100,
                packed_size: 100,
                compressed: false,
                data: None,
            }
        }

        #[test]
        fn empty_patterns_returns_all() {
            let entries = vec![make_entry("a.txt"), make_entry("b.txt")];
            let no_patterns: &[String] = &[];
            let (filtered, missing) =
                filter_and_track_patterns(&entries, no_patterns, |entry, pattern| {
                    entry.name.contains(pattern)
                });
            assert_eq!(filtered.len(), 2);
            assert!(missing.is_empty());
        }

        #[test]
        fn filters_matching_entries() {
            let entries = vec![
                make_entry("a.txt"),
                make_entry("b.txt"),
                make_entry("c.dat"),
            ];
            let patterns = vec!["a.txt".to_string()];
            let (filtered, missing) =
                filter_and_track_patterns(&entries, &patterns, |entry, pattern| {
                    entry.name.contains(pattern)
                });
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].name, "a.txt");
            assert!(missing.is_empty());
        }

        #[test]
        fn reports_missing_patterns() {
            let entries = vec![make_entry("a.txt")];
            let patterns = vec!["missing.txt".to_string()];
            let (filtered, missing) =
                filter_and_track_patterns(&entries, &patterns, |entry, pattern| {
                    entry.name.contains(pattern)
                });
            assert!(filtered.is_empty());
            assert_eq!(missing, vec!["missing.txt"]);
        }

        #[test]
        fn no_duplicate_matches() {
            let entries = vec![make_entry("abc.txt")];
            // Both patterns match the same entry
            let patterns = vec!["abc".to_string(), "txt".to_string()];
            let (filtered, _) = filter_and_track_patterns(&entries, &patterns, |entry, pattern| {
                entry.name.contains(pattern)
            });
            // Should only appear once (matched by first pattern)
            assert_eq!(filtered.len(), 1);
        }

        #[test]
        fn every_pattern_that_matches_counts_as_found() {
            // `l a.dat '*.TXT' FOO.TXT` must not report FOO.TXT missing just
            // because the glob already selected it.
            let entries = vec![make_entry("abc.txt")];
            let patterns = vec!["abc".to_string(), "txt".to_string()];
            let (filtered, missing) =
                filter_and_track_patterns(&entries, &patterns, |entry, pattern| {
                    entry.name.contains(pattern)
                });
            assert_eq!(filtered.len(), 1);
            assert!(missing.is_empty(), "reported missing: {missing:?}");
        }
    }

    // ── Path traversal protection ──────────────────────────────────

    mod path_traversal {
        use super::*;
        use crate::archive::DatArchive;

        #[test]
        fn rejects_dot_dot_in_path() {
            assert!(utils::validate_archive_path("../etc/passwd").is_err());
        }

        #[test]
        fn rejects_dot_dot_with_backslashes() {
            assert!(utils::validate_archive_path("..\\etc\\passwd").is_err());
        }

        #[test]
        fn rejects_embedded_dot_dot() {
            assert!(utils::validate_archive_path("art/../../etc/passwd").is_err());
        }

        #[test]
        fn rejects_trailing_dot_dot() {
            assert!(utils::validate_archive_path("art/critters/..").is_err());
        }

        #[test]
        fn accepts_normal_path() {
            assert!(utils::validate_archive_path("art/critters/file.frm").is_ok());
        }

        #[test]
        fn accepts_path_with_dots_in_filename() {
            assert!(utils::validate_archive_path("art/file.v2.0.frm").is_ok());
        }

        #[test]
        fn accepts_dotfile() {
            assert!(utils::validate_archive_path("art/.hidden").is_ok());
        }

        #[test]
        fn accepts_single_dot_component() {
            // "." as a component is harmless (current directory)
            assert!(utils::validate_archive_path("art/./file.frm").is_ok());
        }

        #[test]
        fn rejects_dot_dot_only() {
            assert!(utils::validate_archive_path("..").is_err());
        }

        fn unsafe_reason(path: &str) -> String {
            let err = utils::validate_archive_path(path).unwrap_err();
            format!("{err}")
        }

        /// On Windows `CON` opens the console and `NUL` discards data, with or
        /// without an extension, whatever directory they sit in.
        #[test]
        fn rejects_a_windows_device_name_in_any_case_and_with_any_extension() {
            for path in ["art/CON", "art/con.frm", "Nul.txt", "art/Com1.acm", "LPT9"] {
                let reason = unsafe_reason(path);
                assert!(
                    reason.starts_with("Unsafe path in archive entry (reserved device name"),
                    "{path}: {reason}"
                );
            }
        }

        #[test]
        fn accepts_names_that_only_begin_like_a_device() {
            for path in ["art/CONSOLE.TXT", "COM10.ACM", "art/AUXILIARY", "NULL.DAT"] {
                assert!(utils::validate_archive_path(path).is_ok(), "{path}");
            }
        }

        /// `name:stream` writes an NTFS alternate data stream instead of a file.
        #[test]
        fn rejects_a_colon_inside_a_name() {
            assert_eq!(
                unsafe_reason("art/file.txt:hidden"),
                format!(
                    "Unsafe path in archive entry (':' in name 'file.txt:hidden'): {}",
                    utils::normalize_path_for_display("art/file.txt:hidden")
                )
            );
        }

        /// Windows strips a trailing dot or space, so the entry would land on a
        /// different name than the one listed.
        #[test]
        fn rejects_a_trailing_dot_or_space() {
            for path in ["art/file.", "art/dir /x.txt"] {
                let reason = unsafe_reason(path);
                assert!(
                    reason.starts_with("Unsafe path in archive entry (trailing dot or space"),
                    "{path}: {reason}"
                );
            }
        }

        /// `Path::join` replaces the base when its argument is absolute, so an
        /// entry stored with a leading separator escapes `-o` entirely.
        #[test]
        fn rejects_an_absolute_entry_on_extraction() {
            assert!(utils::validate_archive_path("\\tmp\\x.txt").is_err());
            assert!(utils::validate_archive_path("/tmp/x.txt").is_err());
        }

        /// Rejected on every host, not only Windows: `Component::Prefix` is
        /// parsed only by the Windows implementation, so a drive-prefixed entry
        /// reaches a Linux extractor as an ordinary component and would then
        /// escape when the same archive is extracted on Windows.
        #[test]
        fn rejects_a_drive_prefixed_entry_on_extraction() {
            assert!(utils::validate_archive_path("C:\\x.txt").is_err());
            assert!(utils::validate_archive_path("c:/x.txt").is_err());
        }

        /// The add path shares the extract path's walk, so it shares this too.
        #[test]
        fn rejects_a_drive_prefixed_path_on_add() {
            assert!(utils::validate_add_archive_path("C:\\x.txt").is_err());
        }

        /// A colon inside a name is not a drive prefix: it is still refused, but as
        /// a stream separator, not misreported as an absolute path.
        #[test]
        fn reports_a_colon_inside_a_component_as_a_colon_not_a_drive() {
            for path in ["art/od:d.frm", "CC:/x.txt"] {
                let err = utils::validate_archive_path(path).unwrap_err().to_string();
                assert!(err.contains("':' in name"), "{path}: {err}");
                assert!(!err.contains("drive prefix"), "{path}: {err}");
            }
        }

        /// The consumer-level guard for the same defect: extracting an archive
        /// whose entry is stored absolute must fail loudly and leave nothing at
        /// the path the entry names. Asserting the validator alone would not
        /// prove the extract path calls it.
        #[test]
        fn extraction_writes_nothing_outside_the_output_directory() {
            let pid = std::process::id();
            let escape = std::env::temp_dir().join(format!("dat3_escape_{pid}.txt"));
            std::fs::remove_file(&escape).ok();
            assert!(!escape.exists(), "stale probe file from an earlier run");

            // Stored the way a hostile archive would: a leading separator, which
            // is what makes Path::join discard the output directory.
            let entry = format!("\\tmp\\dat3_escape_{pid}.txt");
            let archive_path = std::env::temp_dir().join(format!("dat3_escape_src_{pid}.dat"));
            std::fs::write(&archive_path, super::dat1_bytes(46, 1, &entry)).unwrap();

            let out = std::env::temp_dir().join(format!("dat3_escape_out_{pid}"));
            let _ = std::fs::remove_dir_all(&out);
            let archive = DatArchive::open(&archive_path).unwrap();
            let result = archive.extract(
                &out,
                ExtractionMode::PreserveStructure,
                &crate::test_support::exact(&[], MissingFiles::Fail),
            );

            let escaped = escape.exists();
            std::fs::remove_file(&archive_path).ok();
            std::fs::remove_file(&escape).ok();
            let _ = std::fs::remove_dir_all(&out);

            assert!(!escaped, "extraction wrote outside the output directory");
            assert!(result.is_err(), "extraction of a hostile entry should fail");
        }

        #[test]
        fn normalizes_single_dot_component_for_added_archive_paths() {
            assert_eq!(
                utils::validate_add_archive_path("art/./file.frm").unwrap(),
                "art/file.frm"
            );
        }

        #[test]
        fn normalizes_leading_dot_slash() {
            assert_eq!(utils::validate_add_archive_path("./foo").unwrap(), "foo");
        }

        #[test]
        fn normalizes_internal_dot_component() {
            assert_eq!(
                utils::validate_add_archive_path("foo/./bar").unwrap(),
                "foo/bar"
            );
        }

        #[test]
        fn rejects_dot_only_as_empty() {
            assert!(utils::validate_add_archive_path(".").is_err());
        }

        #[test]
        fn rejects_dot_slash_dot_as_empty() {
            assert!(utils::validate_add_archive_path("./.").is_err());
        }

        #[test]
        fn rejects_absolute_path_for_added_archive_paths() {
            assert!(utils::validate_add_archive_path("/art/file.frm").is_err());
        }
    }

    // ── calculate_archive_path ─────────────────────────────────────

    mod calculate_archive_path {
        use super::*;

        #[test]
        fn single_file_no_target() {
            let result = utils::calculate_archive_path(
                Path::new("file.txt"),
                Path::new("file.txt"),
                None,
                None,
            )
            .unwrap();
            assert_eq!(result, "file.txt");
        }

        #[test]
        fn single_file_with_target() {
            let result = utils::calculate_archive_path(
                Path::new("file.txt"),
                Path::new("file.txt"),
                Some("data"),
                None,
            )
            .unwrap();
            assert_eq!(result, "data\\file.txt");
        }

        #[test]
        fn strip_leading_directory() {
            let result = utils::calculate_archive_path(
                Path::new("patch000/file.txt"),
                Path::new("patch000/file.txt"),
                None,
                None,
            )
            .unwrap();
            assert_eq!(result, "patch000\\file.txt");
        }

        #[test]
        fn dot_slash_prefix_is_only_normalized() {
            let result = utils::calculate_archive_path(
                Path::new("./patch000/file.txt"),
                Path::new("./patch000/file.txt"),
                None,
                None,
            )
            .unwrap();
            assert_eq!(result, "patch000\\file.txt");
        }

        #[test]
        fn result_uses_backslashes() {
            let result = utils::calculate_archive_path(
                Path::new("art/critters/file.frm"),
                Path::new("art/critters/file.frm"),
                None,
                None,
            )
            .unwrap();
            // Should use backslashes (archive format)
            assert!(result.contains('\\') || !result.contains('/'));
        }

        #[test]
        fn absolute_unix_path_strips_root_only() {
            let result = utils::calculate_archive_path(
                Path::new("/patch000/file.txt"),
                Path::new("/patch000/file.txt"),
                None,
                None,
            )
            .unwrap();
            assert_eq!(result, "patch000\\file.txt");
        }

        #[test]
        fn change_dir_becomes_archive_root() {
            let result = utils::calculate_archive_path(
                Path::new("mods/patch000/file.txt"),
                Path::new("mods/patch000/file.txt"),
                None,
                Some(Path::new("mods")),
            )
            .unwrap();
            assert_eq!(result, "patch000\\file.txt");
        }

        #[test]
        fn change_dir_resolution_happens_before_target_dir() {
            let result = utils::calculate_archive_path(
                Path::new("mods/patch000/file.txt"),
                Path::new("mods/patch000/file.txt"),
                Some("data"),
                Some(Path::new("mods")),
            )
            .unwrap();
            assert_eq!(result, "data\\patch000\\file.txt");
        }

        #[test]
        fn normalizes_internal_dot_component() {
            let result = utils::calculate_archive_path(
                Path::new("art/./file.frm"),
                Path::new("art/./file.frm"),
                None,
                None,
            )
            .unwrap();
            assert_eq!(result, "art\\file.frm");
        }

        #[test]
        fn rejects_file_outside_change_dir() {
            let result = utils::calculate_archive_path(
                Path::new("outside/file.txt"),
                Path::new("outside/file.txt"),
                None,
                Some(Path::new("mods")),
            );
            assert!(result.is_err());
        }

        #[cfg(windows)]
        #[test]
        fn absolute_windows_path_strips_drive_only() {
            let result = utils::calculate_archive_path(
                Path::new(r"C:\patch000\file.txt"),
                Path::new(r"C:\patch000\file.txt"),
                None,
                None,
            )
            .unwrap();
            assert_eq!(result, "patch000\\file.txt");
        }
    }

    // ── resolve_add_input_path ────────────────────────────────────

    mod resolve_add_input_path {
        use super::*;
        use std::fs;
        use std::path::PathBuf;

        #[test]
        fn rejects_absolute_path_outside_change_dir() {
            let parent = crate::test_support::ScratchPath::dir("abs-outside");
            let mods = parent.join("mods");
            fs::create_dir_all(&mods).unwrap();
            let outside = parent.join("outside.txt");
            fs::write(&outside, b"secret").unwrap();

            let result = utils::resolve_add_input_path(&outside, Some(&mods));

            assert!(
                result.is_err(),
                "absolute path outside change_dir must be rejected"
            );
        }

        /// RAII guard that restores the original working directory on drop.
        /// Ensures test cleanup even if the test panics.
        struct CurrentDirGuard {
            original: PathBuf,
        }

        impl CurrentDirGuard {
            fn enter(new_dir: &std::path::Path) -> Self {
                let original = std::env::current_dir().unwrap();
                std::env::set_current_dir(new_dir).unwrap();
                Self { original }
            }
        }

        impl Drop for CurrentDirGuard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.original);
            }
        }

        fn make_temp_dir(name: &str) -> crate::test_support::ScratchPath {
            crate::test_support::ScratchPath::dir(name)
        }

        #[test]
        fn resolves_relative_path_inside_change_dir() {
            let root = make_temp_dir("resolve-inside");
            let file = root.join("patch000/file.txt");
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, b"test").unwrap();

            let resolved =
                utils::resolve_add_input_path(Path::new("patch000/file.txt"), Some(&root)).unwrap();

            assert_eq!(resolved, fs::canonicalize(file).unwrap());
        }

        #[test]
        fn allows_absolute_path_inside_change_dir() {
            let root = make_temp_dir("resolve-absolute");
            let file = root.join("patch000/file.txt");
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, b"test").unwrap();

            let resolved = utils::resolve_add_input_path(&file, Some(&root)).unwrap();

            assert_eq!(resolved, fs::canonicalize(file).unwrap());
        }

        #[test]
        fn leaves_relative_path_unchanged_without_change_dir() {
            let root = make_temp_dir("resolve-relative");
            let file = root.join("patch000/file.txt");
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, b"test").unwrap();

            let _guard = CurrentDirGuard::enter(&root);
            let resolved =
                utils::resolve_add_input_path(Path::new("patch000/file.txt"), None).unwrap();

            assert_eq!(resolved, Path::new("patch000/file.txt"));
        }

        #[test]
        fn rejects_parent_traversal_outside_change_dir() {
            let parent = make_temp_dir("resolve-parent");
            let root = parent.join("mods");
            fs::create_dir_all(&root).unwrap();
            let outside = parent.join("outside.txt");
            fs::write(&outside, b"test").unwrap();

            let result = utils::resolve_add_input_path(Path::new("../outside.txt"), Some(&root));

            assert!(result.is_err());
        }

        #[cfg(unix)]
        #[test]
        fn rejects_symlinked_file_inside_change_dir() {
            use std::os::unix::fs::symlink;

            let root = make_temp_dir("resolve-symlink-file");
            let real_file = root.join("real.txt");
            let link_file = root.join("link.txt");
            fs::write(&real_file, b"real").unwrap();
            symlink(&real_file, &link_file).unwrap();

            let result = utils::resolve_add_input_path(Path::new("link.txt"), Some(&root));

            assert!(result.is_err());
        }

        #[cfg(unix)]
        #[test]
        fn rejects_symlinked_directory_inside_change_dir() {
            use std::os::unix::fs::symlink;

            let root = make_temp_dir("resolve-symlink-dir");
            let real_dir = root.join("real_dir");
            let link_dir = root.join("link_dir");
            fs::create_dir(&real_dir).unwrap();
            symlink(&real_dir, &link_dir).unwrap();

            let result = utils::resolve_add_input_path(Path::new("link_dir"), Some(&root));

            assert!(result.is_err());
        }

        #[cfg(unix)]
        #[test]
        fn rejects_symlink_to_outside_via_absolute_path() {
            use std::os::unix::fs::symlink;

            let parent = make_temp_dir("resolve-symlink-abs");
            let root = parent.join("mods");
            let outside = parent.join("secret.txt");
            fs::create_dir_all(&root).unwrap();
            fs::write(&outside, b"secret").unwrap();

            // Create symlink inside root that points to file outside
            let link_in_root = root.join("link_to_outside");
            symlink(&outside, &link_in_root).unwrap();

            // Using absolute path to the symlink should still reject it
            let result = utils::resolve_add_input_path(&link_in_root, Some(&root));

            assert!(result.is_err());
        }
    }

    // ── collect_files ─────────────────────────────────────────────

    mod collect_files {
        use super::*;
        use std::fs;

        fn make_temp_dir(name: &str) -> crate::test_support::ScratchPath {
            crate::test_support::ScratchPath::dir(name)
        }

        #[cfg(unix)]
        #[test]
        fn reports_dangling_symlink() {
            // A dangling symlink (target does not exist) must be skipped silently;
            // the message includes the word "dangling" to distinguish it from a
            // non-dangling symlink skip.
            use std::os::unix::fs::symlink;

            let root = make_temp_dir("collect-dangling");
            let real_file = root.join("real.txt");
            fs::write(&real_file, b"real").unwrap();

            // Create a symlink whose target does not exist
            let dangling = root.join("dangling.txt");
            symlink(root.join("nonexistent.txt"), &dangling).unwrap();

            let files = utils::collect_files(&root).unwrap();

            // The dangling symlink must be skipped; only real.txt is collected
            assert_eq!(files.len(), 1);
            assert!(files[0].ends_with("real.txt"));
        }

        #[test]
        fn rejects_non_ascii_in_nested_subdir() {
            // Files with non-ASCII names inside nested subdirectories must cause an error.
            let root = make_temp_dir("collect-nonascii");
            let sub = root.join("sub");
            fs::create_dir_all(&sub).unwrap();

            // Write a file whose path will contain a non-ASCII character.
            // We use a byte-level approach to create the file since the name itself
            // is non-ASCII on the filesystem.
            let non_ascii_name = "caf\u{00e9}.txt"; // "café.txt"
            let file = sub.join(non_ascii_name);
            fs::write(&file, b"content").unwrap();

            let result = utils::collect_files(&root);

            assert!(
                result.is_err(),
                "non-ASCII filename in nested subdir must be rejected"
            );
        }

        #[cfg(unix)]
        #[test]
        fn skips_symlinked_files_and_directories() {
            use std::os::unix::fs::symlink;

            let root = make_temp_dir("collect-symlinks");
            let real_file = root.join("patch000/real.txt");
            let outside_dir = root.join("outside");
            let outside_file = outside_dir.join("secret.txt");
            let file_link = root.join("patch000/link.txt");
            let dir_link = root.join("patch000/linkdir");

            fs::create_dir_all(real_file.parent().unwrap()).unwrap();
            fs::create_dir_all(&outside_dir).unwrap();
            fs::write(&real_file, b"real").unwrap();
            fs::write(&outside_file, b"secret").unwrap();
            symlink(&outside_file, &file_link).unwrap();
            symlink(&outside_dir, &dir_link).unwrap();

            let files = utils::collect_files(root.join("patch000")).unwrap();

            assert_eq!(files, vec![real_file]);
        }
    }

    // ── expand_response_files_with_stripping ──────────────────────

    mod expand_response_files_with_stripping {
        use super::*;
        use std::fs;

        fn make_temp_dir(name: &str) -> crate::test_support::ScratchPath {
            crate::test_support::ScratchPath::dir(name)
        }

        #[test]
        fn dot_slash_prefix_does_not_enable_directory_stripping() {
            let expanded =
                utils::expand_response_files_with_stripping(&["./patch000/file.txt".into()], None)
                    .unwrap();

            assert_eq!(expanded, vec![Path::new("./patch000/file.txt")]);
        }

        #[test]
        fn allows_absolute_path_inside_change_dir() {
            let root = make_temp_dir("expand-abs-inside");
            let file = root.join("file.txt");
            fs::write(&file, b"test").unwrap();

            let expanded = utils::expand_response_files_with_stripping(
                &[file.to_str().unwrap().into()],
                Some(&root),
            )
            .unwrap();

            assert_eq!(expanded, vec![file]);
        }

        #[test]
        fn passes_through_absolute_path_outside_change_dir() {
            // expand_response_files_with_stripping no longer validates bounds —
            // that is deferred to resolve_add_input_path (the sole security gate).
            // The function must return the absolute path unchanged.
            let parent = make_temp_dir("expand-abs-outside");
            let root = parent.join("mods");
            let outside = parent.join("secret.txt");
            fs::create_dir_all(&root).unwrap();
            fs::write(&outside, b"secret").unwrap();

            let result = utils::expand_response_files_with_stripping(
                &[outside.to_str().unwrap().into()],
                Some(&root),
            );

            // expand_response_files_with_stripping should succeed; security
            // enforcement happens in resolve_add_input_path
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), vec![outside]);
        }

        #[test]
        fn passes_through_absolute_path_not_in_change_dir() {
            // Same as above: absolute path resolution is deferred to
            // resolve_add_input_path, not done here.
            let root = make_temp_dir("expand-abs-parent");

            // /etc/passwd may not exist; use a path that does to avoid NotFound
            // inside glob expansion (no glob here, so it just passes through).
            let result =
                utils::expand_response_files_with_stripping(&["/etc/passwd".into()], Some(&root));

            // No longer rejected at this stage — validation is in resolve_add_input_path
            assert!(result.is_ok());
            assert_eq!(
                result.unwrap(),
                vec![std::path::PathBuf::from("/etc/passwd")]
            );
        }
    }

    // ── to_system_path ─────────────────────────────────────────────

    mod to_system_path {
        use super::*;

        #[test]
        fn converts_backslashes_to_system_separator() {
            let result = utils::to_system_path("ART\\CRITTERS\\FILE.FRM");
            let expected = std::path::PathBuf::from(
                "ART/CRITTERS/FILE.FRM".replace('/', std::path::MAIN_SEPARATOR_STR),
            );
            assert_eq!(result, expected);
        }
    }

    // ── FileEntry constructors ─────────────────────────────────────

    mod file_entry {
        use super::*;

        #[test]
        fn with_data_sets_packed_size() {
            let data = vec![1, 2, 3, 4, 5];
            let entry = FileEntry::with_data("test.txt".to_string(), data, false);
            assert_eq!(entry.packed_size, 5);
            assert_eq!(entry.offset, 0);
            assert!(!entry.compressed);
        }

        #[test]
        fn with_compression_data_tracks_both_sizes() {
            let original = vec![1, 2, 3, 4, 5, 6, 7, 8];
            let compressed = vec![1, 2, 3];
            let entry =
                FileEntry::with_compression_data("test.txt".to_string(), original, compressed);
            assert_eq!(entry.size, 8);
            assert_eq!(entry.packed_size, 3);
            assert!(entry.compressed);
        }
    }

    // ── normalize_user_patterns ────────────────────────────────────

    mod normalize_user_patterns {
        use super::*;

        #[test]
        fn converts_all_patterns() {
            let patterns = vec![
                "art/critters/file.frm".to_string(),
                "sound\\music.acm".to_string(),
            ];
            let normalized = utils::normalize_user_patterns(&patterns);
            assert_eq!(normalized[0], "art\\critters\\file.frm");
            assert_eq!(normalized[1], "sound\\music.acm");
        }

        #[test]
        fn empty_patterns() {
            let normalized = utils::normalize_user_patterns(&[]);
            assert!(normalized.is_empty());
        }
    }

    // ── Missing-name policy ────────────────────────────────────────

    mod missing_files {
        use super::*;

        fn entry(name: &str) -> FileEntry {
            FileEntry {
                name: name.to_string(),
                offset: 0,
                size: 100,
                packed_size: 100,
                compressed: false,
                data: None,
            }
        }

        #[test]
        fn filtering_fails_on_a_missing_pattern_by_default() {
            let entries = vec![entry("a.txt")];
            let patterns = vec!["a.txt".to_string(), "nope.txt".to_string()];
            let err = filter_files_by_patterns(
                &entries,
                &crate::test_support::exact(&patterns, MissingFiles::Fail),
            )
            .unwrap_err();
            assert!(
                err.to_string().contains("not found"),
                "unexpected error: {err}"
            );
        }

        #[test]
        fn filtering_keeps_the_matched_files_when_misses_are_tolerated() {
            let entries = vec![entry("a.txt"), entry("b.txt")];
            let patterns = vec!["a.txt".to_string(), "nope.txt".to_string()];
            let matched = filter_files_by_patterns(
                &entries,
                &crate::test_support::exact(&patterns, MissingFiles::Warn),
            )
            .unwrap();
            assert_eq!(matched.len(), 1);
            assert_eq!(matched[0].name, "a.txt");
        }

        /// Every requested name missing is still tolerated: the caller asked for
        /// whatever is there, and nothing is there.
        #[test]
        fn filtering_succeeds_with_no_matches_at_all_when_misses_are_tolerated() {
            let entries = vec![entry("a.txt")];
            let patterns = vec!["nope.txt".to_string()];
            let matched = filter_files_by_patterns(
                &entries,
                &crate::test_support::exact(&patterns, MissingFiles::Warn),
            )
            .unwrap();
            assert!(matched.is_empty());
        }

        #[test]
        fn listing_fails_on_a_missing_pattern_by_default() {
            let entries = [entry("a.txt")];
            let all: Vec<&FileEntry> = entries.iter().collect();
            let patterns = vec!["nope.txt".to_string()];
            assert!(
                list_files_filtered(
                    &all,
                    &crate::test_support::exact(&patterns, MissingFiles::Fail),
                    ListFormat::Text
                )
                .is_err()
            );
        }

        #[test]
        fn listing_succeeds_when_misses_are_tolerated() {
            let entries = [entry("a.txt")];
            let all: Vec<&FileEntry> = entries.iter().collect();
            let patterns = vec!["a.txt".to_string(), "nope.txt".to_string()];
            assert!(
                list_files_filtered(
                    &all,
                    &crate::test_support::exact(&patterns, MissingFiles::Warn),
                    ListFormat::Text
                )
                .is_ok()
            );
        }
    }

    // ── format_file_listing_json ───────────────────────────────────

    mod format_file_listing_json {
        use super::*;

        fn entry(name: &str, size: u32, packed_size: u32, compressed: bool) -> FileEntry {
            FileEntry {
                name: name.to_string(),
                offset: 0,
                size,
                packed_size,
                compressed,
                data: None,
            }
        }

        #[test]
        fn empty_listing_is_an_empty_array() {
            let files: Vec<FileEntry> = vec![];
            assert_eq!(
                utils::format_file_listing_json(&files, &NameView::new(CaseMode::Sensitive, [])),
                "[]"
            );
        }

        #[test]
        fn renders_one_entry_per_line_with_all_fields() {
            let files = vec![entry("ART\\SPLASH.RIX", 1024, 512, true)];
            assert_eq!(
                utils::format_file_listing_json(&files, &NameView::new(CaseMode::Sensitive, [])),
                "[\n  {\"name\": \"ART/SPLASH.RIX\", \"size\": 1024, \"packed_size\": 512, \
                 \"compressed\": true}\n]"
            );
        }

        #[test]
        fn separates_entries_with_commas() {
            let files = vec![
                entry("a.txt", 1, 1, false),
                entry("b.txt", 2, 2, false),
                entry("c.txt", 3, 3, false),
            ];
            let json =
                utils::format_file_listing_json(&files, &NameView::new(CaseMode::Sensitive, []));
            assert_eq!(json.matches("},\n").count(), 2);
            assert!(json.ends_with("false}\n]"), "no trailing comma: {json}");
        }

        /// Backslashes are the archive's own separator, so an unescaped name
        /// would be the most common way to emit an unparseable document.
        #[test]
        fn always_uses_forward_slashes_regardless_of_platform() {
            let files = vec![entry("ART\\CRITTERS\\HANPWRAA.FRM", 10, 10, false)];
            let json =
                utils::format_file_listing_json(&files, &NameView::new(CaseMode::Sensitive, []));
            assert!(json.contains("\"ART/CRITTERS/HANPWRAA.FRM\""), "{json}");
            assert!(!json.contains('\\'), "{json}");
        }

        /// Names come from a third-party archive, so they are not trusted to be
        /// JSON-safe.
        #[test]
        fn escapes_characters_that_would_break_the_document() {
            let files = vec![entry("say \"hi\"\n\tx\u{01}.txt", 0, 0, false)];
            let json =
                utils::format_file_listing_json(&files, &NameView::new(CaseMode::Sensitive, []));
            assert!(json.contains(r#"\"hi\""#), "{json}");
            assert!(json.contains("\\n\\tx\\u0001.txt"), "{json}");
        }

        #[test]
        fn passes_through_non_ascii_unescaped() {
            let files = vec![entry("Кириллица.txt", 0, 0, false)];
            let json =
                utils::format_file_listing_json(&files, &NameView::new(CaseMode::Sensitive, []));
            assert!(json.contains("Кириллица.txt"), "{json}");
        }
    }
}
