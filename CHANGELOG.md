# Changelog

## Unreleased

- Fixed: a crafted Fallout 2 or Arcanum archive could make dat3 reserve gigabytes of memory for a single file name, and a crafted entry in any format could expand on extraction far past the size it declares. Entry paths are now limited to 1024 bytes, and an entry must decompress to exactly its declared size.
- Errors for damaged Fallout 1, Fallout 2 and Arcanum archives now report missing data in bytes rather than bits, and name the Fallout 2 entry that failed.
- Fixed: `l`, `x` and `e` reported a requested name as not found, and failed, when an earlier glob in the same command had already selected that file.
- Fixed: adding a file whose name or directory is longer than 255 bytes to a Fallout 1 archive wrote an archive that could not be opened again. The add now fails and leaves the archive untouched.
- Fixed: adding a file of 4 GiB or more wrote a corrupt archive. Such files are now refused.
- A malformed glob pattern (for example an unclosed `[`) is now reported as an error instead of being matched as plain text.
- Fixed: `d` treated glob patterns as literal names and failed with "File not found", although glob deletion was documented. A glob now deletes every entry it matches; a plain name deletes only the entry with that name. If any name or glob matches nothing, nothing is deleted.
- Changed: entry names now ignore case by default, as the games do. Names and globs given to `l`, `x`, `e` and `d` match regardless of case, so `'*.frm'` selects `ART\CRITTERS\A.FRM`. `l` and `l --json` list names in lowercase, `x` and `e` create lowercase files and directories, and `a` stores new entries in lowercase, replacing an entry that differs only in case. Entries already in an archive keep their stored case until they are added again.
- New: `--case-sensitive` matches, lists, extracts and adds names exactly as stored, as earlier releases did.
- Names in an archive that differ only in case (such as `README.TXT` and `readme.txt`) are kept apart: they are listed and extracted in their stored case, a name matching one of them selects all of them, and every command that opens the archive warns about them.
- An Arcanum archive whose entry table marker disagrees with its footer is now reported as damaged when opened, as ToEE archives already were, instead of being read from wherever the footer points.
- Entry names Windows cannot create as named are now refused on every platform, both when extracting and when adding: a `:` inside a name (which writes an NTFS alternate data stream), device names such as `CON`, `NUL`, `COM1` or `LPT1` with or without an extension, and names ending in a dot or space. Extraction checks every name first, so a refused name leaves nothing half-extracted.
- Fixed: when `e` met several files of the same name (ignoring case) in different directories, which copy ended up on disk varied from run to run. The last one in archive order now wins, and `e` warns how many were skipped.
- Fixed: `a` printed each "Skipping symlink" warning twice.
- Fixed: `x` and `e` wrote through a symlink already present at a file's destination, overwriting the file the link pointed to. The link is now replaced by the extracted file.
- Saving an archive now flushes it to disk before replacing the old file, and on Linux and macOS keeps the old file's permissions. Two dat3 runs saving the same archive at once no longer write into each other's temporary file, though the one that finishes last still replaces the other's changes. Archives with very long file names, which could not be saved, now save.
- New: the archive code is available to other Rust programs as the `dat3-core` library, used as a git dependency on this repository. Its API may still change between releases.
- New: releases ship `dat3-wasm.tgz`, an npm package of the same library for Node.js and Electron: open archives from bytes, list, read, add and remove entries, and write the result back out. It runs on every platform, with TypeScript types.
- Changed: `.bgforge.yml` sets the default format with a single flat key, `dat3.default_format: arcanum`. The nested form (`dat3:` with `default_format:` under it) is no longer read, so a config using it falls back to `dat2` until it is rewritten.

## v0.10.1

- New: `--ignore-missing` for `l`, `x` and `e` reports requested names and globs that are not in the archive as a warning instead of failing: whatever matched is still listed or extracted, and the exit status stays 0.

## v0.10.0

- New: support for The Temple of Elemental Evil (Troika) DAT archives - auto-detected on open for listing, extraction, adding, and deleting; `a --format toee` creates a new ToEE archive, and `.bgforge.yml` accepts `dat3.default_format: toee`.

## v0.9.1

- New: releases now ship a `SHA256SUMS` file, so downloads can be verified.
- Adding files to a Fallout 1 archive is much faster, increasingly so on larger archives.
- Fixed: extracting a Fallout 1 archive could stop partway through with an error. `master.dat` now extracts completely.
- Fixed: many Fallout 1 archives could not be opened at all, among them the Fallout 1 demo's.
- Fixed: a crafted archive could write files outside the output directory on extraction.
- Fixed: `x`, `e`, `a` and `d` crashed when their output was piped to a program that exits early, such as `head`.
- Fixed: Fallout 1 archives created by dat3 contained an extra empty directory.

## v0.9.0

- New: `l --json` prints the listing as a JSON array (`name`, `size`, `packed_size`, `compressed`) instead of aligned columns, for tools that consume dat3's output. Paths always use forward slashes, so the same archive lists identically on every platform.
- New: releases now ship a static Linux arm64 binary (`dat3-arm64`) and a WebAssembly build (`dat3.wasm`), which runs under a WASI runtime such as wasmtime or Node.
- Breaking: `x` and `e` (extract) now fail when a requested file or glob matches nothing in the archive, the way `l` (list) already did. The missing names are printed and nothing is extracted; previously they were ignored silently and the exit code was 0.

## v0.8.0

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
