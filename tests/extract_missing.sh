#!/bin/bash

set -xeu -o pipefail

# Test that x and e fail on files that are not in the archive, like l does

# shellcheck source=tests/common.sh
source "$(dirname "$0")/common.sh"

TEST_DIR="$SCRIPT_DIR/test_extract_missing"

rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

mkdir -p src
echo "content a" >src/a.txt
echo "content b" >src/b.txt
"$DAT3" a test.dat -C src a.txt b.txt

# Test 1: extracting an existing file succeeds
"$DAT3" x test.dat -o out_ok a.txt
verify_file out_ok/a.txt

# Test 2: x with a missing file fails
if "$DAT3" x test.dat -o out_missing nope.txt; then
	echo "Error: x should have failed for a file not in the archive"
	exit 1
fi

# Test 3: e with a missing file fails
if "$DAT3" e test.dat -o out_missing_flat nope.txt; then
	echo "Error: e should have failed for a file not in the archive"
	exit 1
fi

# Test 4: x reports the missing name
# Captured to a file: piping into grep under pipefail would report the failing
# dat3 exit status, not whether the name was printed.
"$DAT3" x test.dat -o out_report nope.txt >out_report.log 2>&1 || true
if ! grep -q "nope.txt" out_report.log; then
	echo "Error: x should name the missing file"
	exit 1
fi

# Test 5: a missing file fails the whole extraction, extracting nothing
if "$DAT3" x test.dat -o out_partial a.txt nope.txt; then
	echo "Error: x should have failed when only some files match"
	exit 1
fi
if [ -e out_partial/a.txt ]; then
	echo "Error: x should not extract anything when a requested file is missing"
	exit 1
fi

# Test 6: a glob matching nothing fails, like it does for l
if "$DAT3" x test.dat -o out_glob '*.zzz'; then
	echo "Error: x should have failed for a glob matching no files"
	exit 1
fi

# Test 7: a glob that matches still succeeds
"$DAT3" x test.dat -o out_glob_ok '*.txt'
verify_file out_glob_ok/a.txt
verify_file out_glob_ok/b.txt

# Test 8: extracting the whole archive still succeeds
"$DAT3" x test.dat -o out_all
verify_file out_all/a.txt
verify_file out_all/b.txt

echo "All extract missing-file tests passed!"
