#!/bin/bash

set -xeuo pipefail

# Work inside tests directory
cd "$(dirname "$0")"

# Use static build
DAT3="../target/x86_64-unknown-linux-musl/release/dat3"

# File constants
RPU_DAT="rpu.dat"
RPU2_DAT="rpu2.dat"
RPU_DIR="rpu"
RPU2_DIR="rpu2"

# Helper function to run wine dat2.exe quietly
dat2() {
    WINEDEBUG=-all wine dat2.exe "$@" 2>/dev/null
}

# Helper function to verify file exists
verify_file() {
    if [ ! -f "$1" ]; then
        echo "Error: Expected file not found: $1"
        exit 1
    fi
}

# Download RPU zip if not present
if [ ! -f "rpu_v2.4.33.zip" ]; then
    wget -O rpu_v2.4.33.zip https://github.com/BGforgeNet/Fallout2_Restoration_Project/releases/download/v2.4.33/rpu_v2.4.33.zip
fi

# Extract rpu.dat if not present
if [ ! -f "$RPU_DAT" ]; then
    unzip -j rpu_v2.4.33.zip "mods/$RPU_DAT"
fi

# Verify MD5 checksum
EXPECTED_MD5="80fb4ba2bf94dfd60aeb89851400aefb"
ACTUAL_MD5=$(md5sum "$RPU_DAT" | cut -d' ' -f1)
[ "$ACTUAL_MD5" = "$EXPECTED_MD5" ]

# Test listing files
$DAT3 l "$RPU_DAT"

# Test extraction and verify integrity
rm -rf "$RPU_DIR"
$DAT3 x "$RPU_DAT" -o "$RPU_DIR"

# Generate and compare checksums
cd "$RPU_DIR"
find . -type f -print0 | xargs -0 md5sum > ../rpu2.md5
cd ..
diff -u rpu.md5 rpu2.md5

# Test compression - create new DAT from extracted files  
# DAT2 format with automatic recursive directory structure preservation
rm -f "$RPU2_DAT"
cd "$RPU_DIR"
"../$DAT3" a "../$RPU2_DAT" ./*
cd ..

# Test with original dat2.exe via wine
rm -rf "$RPU2_DIR"
dat2 x -d "$RPU2_DIR" "$RPU2_DAT"

# Compare extracted files from both tools
cd "$RPU2_DIR"
find . -type f -print0 | xargs -0 md5sum > ../rpu2_final.md5
cd ..
diff -u rpu.md5 rpu2_final.md5

# Test adding dummy files to existing archive
echo "dummy content" > dummy1.txt
$DAT3 a "$RPU2_DAT" dummy1.txt

echo "subdirectory dummy content" > dummy2.txt
$DAT3 a "$RPU2_DAT" -t subdir dummy2.txt

# Define dummy file paths
DUMMY1_LINUX="dummy1.txt"
DUMMY1_WINDOWS="dummy1.txt"
DUMMY2_LINUX="subdir/dummy2.txt"
DUMMY2_WINDOWS="subdir\\\\dummy2.txt"

# Verify files are present with both dat3 and wine+dat2.exe
echo "Checking both tools show added files..."
$DAT3 l "$RPU2_DAT" "$DUMMY1_LINUX" "$DUMMY2_LINUX"
dat2 l "$RPU2_DAT" | grep -q "$DUMMY1_WINDOWS"
dat2 l "$RPU2_DAT" | grep -q "$DUMMY2_WINDOWS"

# Remove dummy files from archive
$DAT3 d "$RPU2_DAT" "$DUMMY1_LINUX"
$DAT3 d "$RPU2_DAT" "$DUMMY2_LINUX"

# Verify files are no longer present with both dat3 and wine+dat2.exe
echo "Checking both tools no longer show deleted files..."
if $DAT3 l "$RPU2_DAT" "$DUMMY1_LINUX" "$DUMMY2_LINUX" 2>/dev/null; then
    echo "Error: Files should have been deleted but are still present"
    exit 1
fi
if dat2 l "$RPU2_DAT" | grep -q "$DUMMY1_WINDOWS"; then
    echo "Error: $DUMMY1_WINDOWS should have been deleted but is still present"
    exit 1
fi
if dat2 l "$RPU2_DAT" | grep -q "$DUMMY2_WINDOWS"; then
    echo "Error: $DUMMY2_WINDOWS should have been deleted but is still present"
    exit 1
fi

# Clean up dummy files
rm -f dummy1.txt dummy2.txt

# Test list command with response file
$DAT3 l "$RPU_DAT" @response_files.txt

# Test extraction with response file
rm -rf response_test
mkdir response_test
$DAT3 x "$RPU_DAT" -o response_test @response_files.txt

# Verify all three files were extracted
verify_file "response_test/text/english/credits.txt"
verify_file "response_test/text/english/game/stat.msg"
verify_file "response_test/art/critters/critters.lst"

# Test flat extraction with response file
rm -rf response_flat
mkdir response_flat
$DAT3 e "$RPU_DAT" -o response_flat @response_files.txt

# Verify all three files were extracted to flat directory
verify_file "response_flat/credits.txt"
verify_file "response_flat/stat.msg"
verify_file "response_flat/critters.lst"

# Test error case - mixing @response-file with explicit files
if $DAT3 l "$RPU_DAT" @response_files.txt text/english/combat.txt 2>/dev/null; then
    echo "Error: Command should have failed when mixing @response-file with explicit files"
    exit 1
fi

# Clean up response file test
rm -rf response_test response_flat

# Clean up
rm -rf "$RPU_DIR" rpu2.md5 "$RPU2_DAT" "$RPU2_DIR" rpu2_final.md5
