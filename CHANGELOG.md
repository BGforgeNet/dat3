# Changelog

## v0.6.2

- Add-path normalization now strips only `./` / `.\` prefixes and absolute path roots while preserving the first real directory.
- Absolute source paths are stored as relative archive entries instead of unsafe or invalid absolute paths.

## v0.6.1

- Path traversal protection: archive entries with `..` in their path are now rejected on extraction.

## v0.6.0

- Can use globs in list/delete/extract operations.
- Paths in error messages are normalized too.
- No panic on piping output.
- Debug code cleanup.

## v0.5.0

Set sort order to be case-insensitive for windows compatibility.

## v0.4.0

Added globbing capability.

## v0.3.0

Now only accept ASCII filenames.

## v0.2.0

Removed `-r` flag - directories are now always processed recursively.

## v0.1.0

Initial release.
