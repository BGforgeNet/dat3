# DAT3 - Fallout DAT Tool

Fallout and Troika .dat management CLI.

Crossplatform, static Rust re-implementation of DAT2, with minor differences. Also supports Arcanum and The Temple of Elemental Evil .dat archives.

- [Usage](#usage)
- [Differences from DAT2](#differences-from-dat2)
- [Verifying a release](#verifying-a-release)
- [Building](#building)

## Usage

```bash
dat3

Fallout and Troika .dat management CLI

Usage: dat3 <COMMAND>

Commands:
  l     List files in a DAT archive
  x     Extract files preserving directory structure
  e     Extract files flat (no subdirectories)
  a     Add files to a DAT archive
  d     Delete files from a DAT archive
  help  Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

```

### Extract all files

```bash
dat3 x master.dat
```

### Extract all files into directory

```bash
dat3 x master.dat -o ./extracted/
```

### Extract specific files

```bash
# Can use forward or backward slashes
dat3 x master.dat art/critters/HMMAXX.FRM scripts\generic.int

# Extract with glob pattern (quote to prevent shell expansion)
dat3 x master.dat 'art/critters/*.frm'
```

### Extract without directory structure (flat)

```bash
dat3 e master.dat -o ./files/
```

### List files in a DAT archive

```bash
# List all files
dat3 l master.dat

# List specific files. Can use forward or backward slashes. Output always shows OS-native slash.
dat3 l master.dat art/critters/vault.frm text\english\quotes.txt

# List with glob pattern (quote to prevent shell expansion)
dat3 l master.dat 'art/critters/*.frm'

# List files from response file
dat3 l master.dat @files_to_list.txt

# List as JSON, for another program to parse
dat3 l master.dat --json
```

`--json` prints one entry object per line, and always uses forward slashes so the
output is identical on every platform:

```json
[
  {"name": "art/critters/vault.frm", "size": 4096, "packed_size": 1180, "compressed": true},
  {"name": "text/english/quotes.txt", "size": 84, "packed_size": 84, "compressed": false}
]
```

The names it prints are the names `x`, `e`, and `d` accept back. A pattern that
matches nothing is still an error: the array on stdout is empty, the unmatched
name goes to stderr, and the exit status is non-zero.

### Tolerating names that are not in the archive

By default `l`, `x` and `e` fail when a requested name or glob matches nothing,
and extract nothing. `--ignore-missing` turns that failure into a warning:

```bash
# Lists and extracts whatever is there, warns about the rest, exits 0
dat3 l master.dat --ignore-missing art/critters/vault.frm no/such/file.txt
dat3 x master.dat --ignore-missing -o ./extracted/ @files.txt
```

The unmatched names still go to stderr, so a script can log them. Useful when
one file list is run against several archives and only some of them hold each
file. Every name missing is not an error either - nothing is extracted and the
exit status is still 0.

### Response file support

```bash
# Create a file listing files to process
echo "art/critters/vault.frm" > files.txt
echo "text\english\quotes.txt" >> files.txt
echo "scripts/generic.int" >> files.txt

# Use with any command (mutually exclusive with explicit file lists)
dat3 l master.dat @files.txt
dat3 x master.dat @files.txt -o extracted/
dat3 e master.dat @files.txt -o flat/
dat3 a master.dat @files.txt
dat3 d master.dat @files.txt
```

### Add files to a DAT archive

```bash
# Add single file
dat3 a master.dat myfile.txt

# Add files relative to another directory
dat3 a master.dat -C patch000 file.txt  # patch000/file.txt
Adding: file.txt  # Added to archive root

# Add directory (automatically recursive)
dat3 a master.dat myfolder/

# Add with max compression level
dat3 a master.dat largefile.txt -c 9

# Add to specific directory in archive
dat3 a master.dat myfile.txt -t "art/graphics"

# Choose the format for a new archive (dat1, dat2, arcanum, toee; default dat2)
dat3 a newarchive.dat myfiles/ --format dat1
dat3 a newarchive.dat myfiles/ --format arcanum
dat3 a newarchive.dat myfiles/ --format toee

# Add files from response file
dat3 a master.dat @files_to_add.txt
```

#### Add-path normalization

- `./` and `.\` prefixes are removed before storing paths in the archive
- absolute source paths have only their filesystem root/prefix stripped
- the first real directory name is preserved
- `a -C DIR ...` resolves add operands inside `DIR` and stores paths relative to `DIR`
- `a -C DIR ...` rejects `.`/`..` components and paths that resolve outside `DIR`

Examples:

```bash
dat3 a master.dat ./patch000/file.txt
# stores as patch000/file.txt

dat3 a master.dat ./patch000/*
# stores as patch000/...

dat3 a master.dat /tmp/patch000/file.txt
# stores as tmp/patch000/file.txt
```

#### Default format via .bgforge.yml

When `a` creates a new archive and no `--format` is given, an optional `.bgforge.yml` in the current directory picks the default:

```yaml
dat3:
  default_format: arcanum
```

Supported values: `dat1`, `dat2`, `arcanum`, `toee`. An unrecognized value prints a warning and `dat2` is used. An explicit `--format` always wins, and existing archives always keep their format.

### Delete files from archive

```bash
# Delete single file (cross-platform paths supported)
dat3 d master.dat text/english/quotes.txt

# Delete multiple files
dat3 d master.dat file1.txt art\critters\vault.frm

# Delete with glob pattern (quote to prevent shell expansion)
dat3 d master.dat 'art/critters/*.frm'

# Delete files from response file
dat3 d master.dat @files_to_delete.txt
```

## Differences from DAT2

- Directories are always processed recursively.
- Shrink (`k` command) not implemented.
- Flat extraction is a separate command, `e`.
- DAT1 compression (LZSS) not implemented, only decompression. Fallout 1 style .dat files are thus created without compression.
- Glob patterns (`*`, `?`, `[...]`) supported for list/extract/delete.

## Verifying a release

Every release ships a `SHA256SUMS` file covering its binaries. Download it
alongside the assets and check them:

```bash
sha256sum -c SHA256SUMS
```

Only the files you downloaded need to be present; `sha256sum` reports the rest
as missing.

## Building

### Requirements

- Rust 1.87 or newer
- Target-specific toolchains (install as needed)
- `./install-tools.sh` for the pinned tooling, including [Zig](https://ziglang.org/), which the aarch64
  target needs: mimalloc is C, and no aarch64-musl C compiler is packaged for common distros
- Node 24 or newer, to run the integration suite (`./test.sh`): its helpers under `tests/` are TypeScript, run
  by Node's own type stripping. `npm ci && npm run typecheck` typechecks them. Neither is needed to build dat3

### Build

```bash
./build.sh
```

Builds are static.

Binaries will be at:

```bash
target/x86_64-unknown-linux-musl/release/dat3
target/aarch64-unknown-linux-musl/release/dat3
target/x86_64-pc-windows-gnu/release/dat3.exe
target/i686-pc-windows-gnu/release/dat3.exe
target/wasm32-wasip1/release/dat3.wasm
```
