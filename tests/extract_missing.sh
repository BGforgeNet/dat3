#!/bin/bash

set -xeu -o pipefail

# Test that x and e fail on files that are not in the archive, like l does,
# and that --ignore-missing downgrades that failure to a warning for l, x and e

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

# ── --ignore-missing ──────────────────────────────────────────────────

# Test 9: x --ignore-missing extracts what is there and succeeds
"$DAT3" x test.dat -o out_tolerant --ignore-missing a.txt nope.txt >out_tolerant.log 2>&1
verify_file out_tolerant/a.txt
if ! grep -q "nope.txt" out_tolerant.log; then
	echo "Error: x --ignore-missing should warn about the missing file"
	exit 1
fi
if ! grep -qi "warning" out_tolerant.log; then
	echo "Error: x --ignore-missing should label the report as a warning"
	exit 1
fi

# Test 10: e --ignore-missing behaves the same
"$DAT3" e test.dat -o out_tolerant_flat --ignore-missing a.txt nope.txt
verify_file out_tolerant_flat/a.txt

# Test 11: every requested name missing is still exit 0, with nothing extracted
"$DAT3" x test.dat -o out_tolerant_none --ignore-missing nope.txt
if [ -e out_tolerant_none/nope.txt ]; then
	echo "Error: x --ignore-missing invented a file that is not in the archive"
	exit 1
fi

# Test 12: a glob matching nothing is tolerated too
"$DAT3" x test.dat -o out_tolerant_glob --ignore-missing '*.zzz'

# Test 13: l fails on a missing name but tolerates it with --ignore-missing
if "$DAT3" l test.dat nope.txt; then
	echo "Error: l should have failed for a file not in the archive"
	exit 1
fi
"$DAT3" l test.dat --ignore-missing a.txt nope.txt >list_tolerant.log 2>&1
if ! grep -q "a.txt" list_tolerant.log; then
	echo "Error: l --ignore-missing should still list the files that are present"
	exit 1
fi
if ! grep -q "nope.txt" list_tolerant.log; then
	echo "Error: l --ignore-missing should warn about the missing file"
	exit 1
fi

# Test 14: --json listing tolerates a missing name as well
"$DAT3" l test.dat --json --ignore-missing a.txt nope.txt

echo "All extract missing-file tests passed!"
