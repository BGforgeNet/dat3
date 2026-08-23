#!/bin/bash

set -xeu -o pipefail

# Glob pattern handling for the Windows build, run under wine: the same
# patterns as glob_handling.sh, written with backslash separators.

# shellcheck source=tests/common.sh
source "$(dirname "$0")/common.sh"

require_wine

TEST_DIR="$SCRIPT_DIR/test_glob_handling_wine"

# Clean up any previous test
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

echo "=== Testing glob pattern expansion (Windows build) ==="

# Create test directory structure with various file types
mkdir -p patch000/xxx patch000/yyy
echo "content1" >patch000/1.txt
echo "content2" >patch000/2.txt
echo "content3" >patch000/xxx/3.txt
echo "binary_data" >patch000/data.bin
echo "another_file" >patch000/test.dat
echo "nested_file" >patch000/yyy/nested.txt

echo "Directory structure:"
find . -type f | sort

# Copy Windows binary to test directory for Wine (like rpu_wine.sh does)
# Use 32-bit binary for Wine compatibility in CI
WIN_BINARY="$SCRIPT_DIR/../target/i686-pc-windows-gnu/release/dat3.exe"
cp "$WIN_BINARY" dat3.exe

# Helper function to run Windows command via Wine
run_wine() {
	WINEDEBUG=-all wine dat3.exe "$@"
}

# Helper function to verify file exists in archive
verify_file_exists() {
	local archive="$1"
	local file="$2"

	if ! run_wine l "$archive" "$file" >/dev/null 2>&1; then
		printf "ERROR: %s not found in archive\n" "$file"
		exit 1
	fi
}

# Helper function to verify file does NOT exist in archive
verify_file_missing() {
	local archive="$1"
	local file="$2"

	if run_wine l "$archive" "$file" >/dev/null 2>&1; then
		printf "ERROR: %s should not be in archive\n" "$file"
		exit 1
	fi
}

# Test function for a glob pattern
test_glob_pattern() {
	local test_num="$1"
	local test_name="$2"
	local pattern="$3"
	local verify_files="$4"   # space-separated list of files that should exist
	local verify_missing="$5" # space-separated list of files that should NOT exist
	local file win_file

	echo ""
	echo "=== Test $test_num: $test_name ==="
	echo "Testing $test_name: $pattern"
	run_wine a "test${test_num}.dat" "$pattern"
	echo "$test_name archive contents:"
	run_wine l "test${test_num}.dat"

	# Verify files (convert forward slashes to backslashes)
	echo "Verifying $test_name..."
	for file in $verify_files; do
		win_file=${file//\//\\}
		verify_file_exists "test${test_num}.dat" "$win_file"
	done
	for file in $verify_missing; do
		win_file=${file//\//\\}
		verify_file_missing "test${test_num}.dat" "$win_file"
	done
	echo "$test_name verification passed!"
}

# Run all glob pattern tests
test_glob_pattern "1" "Basic glob pattern" \
	'patch000\*.txt' \
	"patch000/1.txt patch000/2.txt" \
	"patch000/xxx/3.txt"

test_glob_pattern "2" "Recursive glob pattern" \
	'patch000\**\*.txt' \
	"patch000/1.txt patch000/2.txt patch000/xxx/3.txt patch000/yyy/nested.txt" \
	""

test_glob_pattern "3" "Character range glob pattern" \
	'patch000\[12].txt' \
	"patch000/1.txt patch000/2.txt" \
	""

test_glob_pattern "4" "Question mark glob pattern" \
	'patch000\?.txt' \
	"patch000/1.txt patch000/2.txt" \
	""

# Test 5: Dot-prefix normalization with .\ prefix
echo ""
echo "=== Test 5: Dot-prefix normalization with .\\ prefix ==="

printf '%s\n' 'Testing dot-prefix normalization: .\patch000\*'
run_wine a test5.dat '.\\patch000\\*'
echo "Dot-prefix normalization archive contents:"
run_wine l test5.dat

# Files should keep their patch000\ prefix
echo "Verifying dot-prefix normalization..."
for file in patch000\\1.txt patch000\\2.txt patch000\\data.bin patch000\\test.dat patch000\\xxx\\3.txt patch000\\yyy\\nested.txt; do
	verify_file_exists "test5.dat" "$file"
done
echo "Dot-prefix normalization verification passed!"

# Test 6: Mixed file type patterns (multiple patterns)
echo ""
echo "=== Test 6: Mixed file type glob patterns ==="

printf '%s\n' 'Testing mixed file types: patch000\*.txt patch000\*.dat patch000\*.bin'
run_wine a test6.dat 'patch000\*.txt' 'patch000\*.dat' 'patch000\*.bin'
echo "Mixed file type glob archive contents:"
run_wine l test6.dat

echo "Verifying mixed file types..."
for file in patch000\\1.txt patch000\\2.txt patch000\\data.bin patch000\\test.dat; do
	verify_file_exists "test6.dat" "$file"
done
echo "Mixed file type glob pattern verification passed!"

# Test 7: Mixed dot-prefix normalization
echo ""
echo "=== Test 7: Mixed dot-prefix normalization ==="

printf '%s\n' 'Testing mixed normalization: patch000\1.txt .\patch000\2.txt patch000\xxx\3.txt'
run_wine a test7.dat 'patch000\1.txt' '.\patch000\2.txt' 'patch000\xxx\3.txt'
echo "Mixed normalization archive contents:"
run_wine l test7.dat

# All files should keep their paths
echo "Verifying mixed normalization..."
verify_file_exists "test7.dat" "patch000\\1.txt"
verify_file_exists "test7.dat" "patch000\\2.txt"
verify_file_exists "test7.dat" "patch000\\xxx\\3.txt"
echo "Mixed normalization verification passed!"

# Test 8: Glob patterns with .\ prefix keep their directory
echo ""
echo "=== Test 8: Glob patterns with .\\ prefix ==="

printf '%s\n' 'Testing glob with .\ prefix: .\patch000\*.txt .\patch000\data.bin'
run_wine a test8.dat '.\\patch000\\*.txt' '.\\patch000\\data.bin'
echo "Glob with .\\ prefix archive contents:"
run_wine l test8.dat

# Dot prefix should be removed but directory preserved
echo "Verifying glob with .\\ prefix..."
verify_file_exists "test8.dat" "patch000\\1.txt"
verify_file_exists "test8.dat" "patch000\\2.txt"
verify_file_exists "test8.dat" "patch000\\data.bin"
echo "Glob with .\\ prefix verification passed!"

echo ""
echo "All Windows glob tests completed successfully!"
