# DAT3 - Fallout DAT Tool

Fallout .dat management cli.

Crossplatform, static Rust re-implementation of DAT2, with minor differences.

- [Usage](#usage)
- [Differences from DAT2](#differences-from-dat2)
- [Building](#building)

## Usage

```bash
dat3

Fallout .dat management cli

Usage: dat3 <COMMAND>

Commands:
  l     List files in a DAT archive (command: l)
  x     Extract files from a DAT archive with directory structure (command: x)
  e     Extract files without creating directories - all files go to one folder (command: e)
  a     Add files to a DAT archive (command: a)
  d     Delete files from a DAT archive (command: d)
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

# List files from response file
dat3 l master.dat @files_to_list.txt
```

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

# Add directory (automatically recursive)
dat3 a master.dat myfolder/

# Add with max compression level
dat3 a master.dat largefile.txt -c 9

# Add to specific directory in archive
dat3 a master.dat myfile.txt -t "art/graphics"

# Force DAT1 format for new archive
dat3 a newarchive.dat myfiles/ --dat1

# Add files from response file
dat3 a master.dat @files_to_add.txt
```

### Delete files from archive

```bash
# Delete single file (cross-platform paths supported)
dat3 d master.dat text/english/quotes.txt

# Delete multiple files
dat3 d master.dat file1.txt art\critters\vault.frm

# Delete files from response file
dat3 d master.dat @files_to_delete.txt
```

Delete only deletes file records. It doesn't reduce archive size.

## Differences from DAT2

- Directories are always processed recursively.
- Shrink (`k` command) not implemented.
- Flat extraction is a separate command, `e`.
- DAT1 compression (LZSS) not implemented, only decompression. Fallout 1 style .dat files are thus created without compression.

## Building

### Requirements

- Rust 1.70 or newer
- Target-specific toolchains (install as needed)

### Build

```bash
./build.sh
```

Builds are static.

Binaries will be at:

```bash
target/x86_64-unknown-linux-musl/release/dat3
target/x86_64-pc-windows-gnu/release/dat3.exe
target/i686-pc-windows-gnu/release/dat3.exe
```
