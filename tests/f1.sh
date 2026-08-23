#!/bin/bash

set -xeu -o pipefail

# This test expects critter.dat from Fallout 1 to be present in f1/ directory.
# The dat2.exe cross-checks live in f1_wine.sh.

# Work inside f1 directory
cd "$(dirname "$0")/f1"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ../common.sh

# Extract the original archive, repack it as DAT1, and extract that: the two
# trees must be identical
rm -rf ART ART-roundtrip critter_test.dat
$DAT3 x critter.dat
$DAT3 a critter_test.dat --format dat1 ART
$DAT3 x critter_test.dat -o ART-roundtrip
diff -qr ART ART-roundtrip/ART

# Clean up
rm -rf ART ART-roundtrip critter_test.dat
