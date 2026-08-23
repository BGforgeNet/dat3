#!/bin/bash

set -xeu -o pipefail

# dat3 against the real RPU archive: list, extract, repack, modify.
# The dat2.exe cross-checks live in rpu_wine.sh.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

RPU2_DAT="rpu2.dat"
RPU_DIR="rpu"
RPU2_DIR="rpu2"

fetch_rpu_dat

# Test listing files
$DAT3 l "$RPU_DAT"

# Test extraction and verify integrity against the recorded checksums
rm -rf "$RPU_DIR"
$DAT3 x "$RPU_DAT" -o "$RPU_DIR"
hash_tree "$RPU_DIR" rpu2.md5
diff -u rpu.md5 rpu2.md5

# Repack the extracted tree and read it back with dat3: every file must
# survive the round trip byte for byte
build_rpu2_dat "$RPU_DIR" "$RPU2_DAT"
rm -rf "$RPU2_DIR"
$DAT3 x "$RPU2_DAT" -o "$RPU2_DIR"
hash_tree "$RPU2_DIR" rpu2_final.md5
diff -u rpu.md5 rpu2_final.md5

# Test adding dummy files to existing archive
echo "dummy content" >dummy1.txt
$DAT3 a "$RPU2_DAT" dummy1.txt

echo "subdirectory dummy content" >dummy2.txt
$DAT3 a "$RPU2_DAT" -t subdir dummy2.txt

DUMMY1="dummy1.txt"
DUMMY2="subdir/dummy2.txt"

echo "Checking added files are listed..."
$DAT3 l "$RPU2_DAT" "$DUMMY1" "$DUMMY2"

# Remove dummy files from archive
$DAT3 d "$RPU2_DAT" "$DUMMY1"
$DAT3 d "$RPU2_DAT" "$DUMMY2"

echo "Checking deleted files are gone..."
if $DAT3 l "$RPU2_DAT" "$DUMMY1" "$DUMMY2" 2>/dev/null; then
	echo "Error: Files should have been deleted but are still present"
	exit 1
fi

# Clean up
rm -f dummy1.txt dummy2.txt
rm -rf "$RPU_DIR" "$RPU2_DIR" rpu2.md5 rpu2_final.md5 "$RPU2_DAT"
