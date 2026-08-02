# Changelog

## Unreleased

- Breaking: the `a` command's `--dat1` flag is replaced by `--format dat1|dat2|arcanum` (default `dat2` for new archives).
- New: an optional `.bgforge.yml` in the current directory (`dat3.default_format: dat1|dat2|arcanum`) sets the default format for newly created archives; `--format` still wins.
- New: support for Arcanum (Troika) DAT archives - auto-detected on open for listing, extraction, adding, and deleting; `a --format arcanum` creates a new Arcanum archive.
- Fallout 1 (DAT1) archives now extract in parallel like DAT2 ones, roughly 1.5-2x faster on multi-core machines. Per-file `Extracting:` lines are replaced by DAT2-style progress reporting.
- Faster DAT2 compression and decompression (new zlib backend): high-compression archive creation is ~17% faster, extraction ~11% faster.
- Fixed: a DAT2 archive containing only empty files could be created but not reopened ("Invalid directory tree position").
- Corrupt or truncated archives now produce a clean error instead of crashing (malformed DAT2 footer) or silently extracting truncated data (damaged DAT1 LZSS streams).
- A crafted size field in an archive can no longer force an outsized upfront memory allocation.
- Creating an archive whose contents exceed the DAT formats' 4 GiB offset limit is now rejected with an error instead of writing a corrupt file.
- Saves are now atomic: an interrupted `a` (add) or `d` (delete) no longer corrupts or destroys the existing archive.
- Saving uses far less memory: archives stream to disk instead of being assembled in RAM first (peak usage down ~60-70%, e.g. 1.9 GiB to 0.6 GiB when creating a DAT1 from 600 MB of files), and saves are 10-35% faster.

## v0.7.0

- Add `-C`/`--change-dir` flag for `a` (add) operation: resolves file operands relative to the given directory and rejects any operand that escapes it or is a symlink.
- `a` (add) now skips all symlinks encountered during directory recursion (previously followed) and no longer errors on dangling symlinks.
- `a` (add) now rejects archive paths containing `..` components, empty paths, or absolute-root/drive prefixes; `.` components are silently normalized away.

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
