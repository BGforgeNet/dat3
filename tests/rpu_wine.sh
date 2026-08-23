#!/bin/bash

set -xeu -o pipefail

# Cross-check the DAT2 format against the original dat2.exe: dat3 writes,
# dat2.exe reads. The dat3-only checks live in rpu.sh.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

require_wine

RPU2_DAT="rpu2.dat"
RPU_DIR="rpu"
RPU2_DIR="rpu2"

# Helper function to run wine dat2.exe quietly
dat2() {
	WINEDEBUG=-all wine dat2.exe "$@" 2>/dev/null
}

fetch_rpu_dat

# Repack the archive with dat3, then extract it with dat2.exe: the checksums
# must match the ones recorded from the original archive
build_rpu2_dat "$RPU_DIR" "$RPU2_DAT"
rm -rf "$RPU2_DIR"
dat2 x -d "$RPU2_DIR" "$RPU2_DAT"
hash_tree "$RPU2_DIR" rpu2_final.md5
diff -u rpu.md5 rpu2_final.md5

# Test adding dummy files to existing archive
echo "dummy content" >dummy1.txt
$DAT3 a "$RPU2_DAT" dummy1.txt

echo "subdirectory dummy content" >dummy2.txt
$DAT3 a "$RPU2_DAT" -t subdir dummy2.txt

# Define dummy file paths
DUMMY1_LINUX="dummy1.txt"
DUMMY1_WINDOWS="dummy1.txt"
DUMMY2_LINUX="subdir/dummy2.txt"
DUMMY2_WINDOWS="subdir\\\\dummy2.txt"

# Verify files are present with both dat3 and wine+dat2.exe.
# dat2.exe's listing goes to a file first: piping it into an early-exiting
# reader would report that reader's signal, not whether the name was found.
echo "Checking both tools show added files..."
$DAT3 l "$RPU2_DAT" "$DUMMY1_LINUX" "$DUMMY2_LINUX"
dat2 l "$RPU2_DAT" >dat2_listing.txt
grep -q "$DUMMY1_WINDOWS" dat2_listing.txt
grep -q "$DUMMY2_WINDOWS" dat2_listing.txt

# Remove dummy files from archive
$DAT3 d "$RPU2_DAT" "$DUMMY1_LINUX"
$DAT3 d "$RPU2_DAT" "$DUMMY2_LINUX"

# Verify files are no longer present with both dat3 and wine+dat2.exe
echo "Checking both tools no longer show deleted files..."
if $DAT3 l "$RPU2_DAT" "$DUMMY1_LINUX" "$DUMMY2_LINUX" 2>/dev/null; then
	echo "Error: Files should have been deleted but are still present"
	exit 1
fi
dat2 l "$RPU2_DAT" >dat2_listing.txt
if grep -q "$DUMMY1_WINDOWS" dat2_listing.txt; then
	echo "Error: $DUMMY1_WINDOWS should have been deleted but is still present"
	exit 1
fi
if grep -q "$DUMMY2_WINDOWS" dat2_listing.txt; then
	echo "Error: $DUMMY2_WINDOWS should have been deleted but is still present"
	exit 1
fi

# Clean up
rm -f dummy1.txt dummy2.txt dat2_listing.txt rpu2_final.md5
rm -rf "$RPU_DIR" "$RPU2_DIR" "$RPU2_DAT"
