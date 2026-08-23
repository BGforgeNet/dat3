#!/bin/bash

set -xeu -o pipefail

# Cross-check the DAT1 format against the original dat2.exe. Expects
# critter.dat from Fallout 1 in the f1/ directory. The dat3-only checks
# live in f1.sh.

# Work inside f1 directory
cd "$(dirname "$0")/f1"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ../common.sh

require_wine

export WINEDEBUG=-all
DAT2="wine ../dat2.exe"
DAT2_ART_DIR="dat2/ART"

if [ ! -d $DAT2_ART_DIR ]; then
	$DAT2 x -d dat2 critter.dat 2>/dev/null
fi

# Test 1: Extract with dat3 and compare with dat2.exe reference extraction
rm -rf ART
$DAT3 x critter.dat
diff -qr $DAT2_ART_DIR ART

# Test 2: Compress with dat3 in DAT1 format, extract with dat2.exe, and compare
# Create new DAT1 archive from extracted files with correct structure
rm -rf ART-roundtrip critter_test.dat
$DAT3 a critter_test.dat --format dat1 ART

# Extract with dat2.exe via wine
$DAT2 x -d ART-roundtrip critter_test.dat 2>/dev/null

# Compare with original reference extraction
diff -qr dat2/ART ART-roundtrip/ART

# Clean up
rm -rf ART ART-roundtrip critter_test.dat
