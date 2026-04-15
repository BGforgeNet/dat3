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

        #[test]
        fn substring_match() {
            assert!(utils::matches_pattern(
                "ART\\CRITTERS\\FILE.FRM",
                "FILE.FRM"
            ));
        }

        #[test]
        fn substring_no_match() {
            assert!(!utils::matches_pattern(
                "ART\\CRITTERS\\FILE.FRM",
                "MISSING.TXT"
            ));
        }

        #[test]
        fn glob_star_matches_extension() {
            assert!(utils::matches_pattern("ART\\CRITTERS\\FILE.FRM", "*.FRM"));
        }

        #[test]
        fn glob_star_no_match_wrong_extension() {
            assert!(!utils::matches_pattern("ART\\CRITTERS\\FILE.FRM", "*.TXT"));
        }

        #[test]
        fn glob_question_mark() {
            assert!(utils::matches_pattern("ART\\CRITTERS\\A.FRM", "?.FRM"));
            assert!(!utils::matches_pattern("ART\\CRITTERS\\AB.FRM", "?.FRM"));
        }

        #[test]
        fn glob_with_path_prefix() {
            assert!(utils::matches_pattern(
                "ART\\CRITTERS\\FILE.FRM",
                "ART/CRITTERS/*.FRM"
            ));
        }

        #[test]
        fn glob_path_no_match_wrong_dir() {
            assert!(!utils::matches_pattern(
                "ART\\CRITTERS\\FILE.FRM",
                "SOUND/*.FRM"
            ));
        }

        #[test]
        fn character_range() {
            assert!(utils::matches_pattern(
                "ART\\CRITTERS\\FILE1.FRM",
                "[A-Z]*.FRM"
            ));
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
            let (filtered, missing) = filter_and_track_patterns(&entries, &[], |entry, pattern| {
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
    }

    // ── Path traversal protection ──────────────────────────────────

    mod path_traversal {
        use super::*;

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
        use std::time::{SystemTime, UNIX_EPOCH};

        #[test]
        fn rejects_absolute_path_outside_change_dir() {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let parent = std::env::temp_dir().join(format!("dat3-abs-outside-{unique}"));
            let mods = parent.join("mods");
            fs::create_dir_all(&mods).unwrap();
            let outside = parent.join("outside.txt");
            fs::write(&outside, b"secret").unwrap();

            let result = utils::resolve_add_input_path(&outside, Some(&mods));

            assert!(
                result.is_err(),
                "absolute path outside change_dir must be rejected"
            );
            fs::remove_dir_all(parent).unwrap();
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

        fn make_temp_dir(name: &str) -> std::path::PathBuf {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("dat3-{name}-{unique}"));
            fs::create_dir_all(&path).unwrap();
            path
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
            fs::remove_dir_all(root).unwrap();
        }

        #[test]
        fn allows_absolute_path_inside_change_dir() {
            let root = make_temp_dir("resolve-absolute");
            let file = root.join("patch000/file.txt");
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, b"test").unwrap();

            let resolved = utils::resolve_add_input_path(&file, Some(&root)).unwrap();

            assert_eq!(resolved, fs::canonicalize(file).unwrap());
            fs::remove_dir_all(root).unwrap();
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
            fs::remove_dir_all(root).unwrap();
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
            fs::remove_dir_all(parent).unwrap();
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
            fs::remove_dir_all(root).unwrap();
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
            fs::remove_dir_all(root).unwrap();
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
            fs::remove_dir_all(parent).unwrap();
        }
    }

    // ── collect_files ─────────────────────────────────────────────

    mod collect_files {
        use super::*;
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        fn make_temp_dir(name: &str) -> std::path::PathBuf {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("dat3-{name}-{unique}"));
            fs::create_dir_all(&path).unwrap();
            path
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

            fs::remove_dir_all(root).unwrap();
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

            fs::remove_dir_all(root).unwrap();
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
            fs::remove_dir_all(root).unwrap();
        }
    }

    // ── expand_response_files_with_stripping ──────────────────────

    mod expand_response_files_with_stripping {
        use super::*;
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        fn make_temp_dir(name: &str) -> std::path::PathBuf {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("dat3-{name}-{unique}"));
            fs::create_dir_all(&path).unwrap();
            path
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
            fs::remove_dir_all(root).unwrap();
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
            fs::remove_dir_all(parent).unwrap();
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
            fs::remove_dir_all(root).unwrap();
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
}
