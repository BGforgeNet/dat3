#!/bin/bash

set -xeu -o pipefail

# Glob pattern handling for the Linux build: adding files by pattern, and
# filtering list/extract by pattern. The Windows-build checks live in
# glob_handling_wine.sh.

# shellcheck source=tests/common.sh
source "$(dirname "$0")/common.sh"

TEST_DIR="$SCRIPT_DIR/test_glob_handling"

# Clean up any previous test
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

echo "=== Testing glob pattern expansion ==="

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

# Helper function to verify file exists in archive
verify_file_exists() {
	local archive="$1"
	local file="$2"

	if ! "$DAT3" l "$archive" "$file" >/dev/null 2>&1; then
		echo "ERROR: $file not found in archive"
		exit 1
	fi
}

# Helper function to verify file does NOT exist in archive
verify_file_missing() {
	local archive="$1"
	local file="$2"

	if "$DAT3" l "$archive" "$file" >/dev/null 2>&1; then
		echo "ERROR: $file should not be in archive"
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
	local file

	echo ""
	echo "=== Test $test_num: $test_name ==="
	echo "Testing $test_name: $pattern"
	"$DAT3" a "test${test_num}.dat" "$pattern"
	echo "$test_name archive contents:"
	"$DAT3" l "test${test_num}.dat"

	echo "Verifying $test_name..."
	for file in $verify_files; do
		verify_file_exists "test${test_num}.dat" "$file"
	done
	for file in $verify_missing; do
		verify_file_missing "test${test_num}.dat" "$file"
	done
	echo "$test_name verification passed!"
}

# Run all glob pattern tests
test_glob_pattern "1" "Basic glob pattern" \
	"patch000/*.txt" \
	"patch000/1.txt patch000/2.txt" \
	"patch000/xxx/3.txt"

test_glob_pattern "2" "Recursive glob pattern" \
	"patch000/**/*.txt" \
	"patch000/1.txt patch000/2.txt patch000/xxx/3.txt patch000/yyy/nested.txt" \
	""

test_glob_pattern "3" "Character range glob pattern" \
	"patch000/[12].txt" \
	"patch000/1.txt patch000/2.txt" \
	""

test_glob_pattern "4" "Question mark glob pattern" \
	"patch000/?.txt" \
	"patch000/1.txt patch000/2.txt" \
	""

# Test 5: Dot-prefix normalization with ./ prefix
echo ""
echo "=== Test 5: Dot-prefix normalization with ./ prefix ==="

echo "Testing dot-prefix normalization: ./patch000/*"
"$DAT3" a test5.dat './patch000/*'
echo "Dot-prefix normalization archive contents:"
"$DAT3" l test5.dat

# Files should keep their patch000/ prefix
echo "Verifying dot-prefix normalization..."
for file in patch000/1.txt patch000/2.txt patch000/data.bin patch000/test.dat patch000/xxx/3.txt patch000/yyy/nested.txt; do
	verify_file_exists "test5.dat" "$file"
done
echo "Dot-prefix normalization verification passed!"

# Test 6: Mixed file type patterns (multiple patterns)
echo ""
echo "=== Test 6: Mixed file type glob patterns ==="

echo "Testing mixed file types: patch000/*.txt patch000/*.dat patch000/*.bin"
"$DAT3" a test6.dat 'patch000/*.txt' 'patch000/*.dat' 'patch000/*.bin'
echo "Mixed file type glob archive contents:"
"$DAT3" l test6.dat

echo "Verifying mixed file types..."
for file in patch000/1.txt patch000/2.txt patch000/data.bin patch000/test.dat; do
	verify_file_exists "test6.dat" "$file"
done
echo "Mixed file type glob pattern verification passed!"

# Test 7: Mixed dot-prefix normalization
echo ""
echo "=== Test 7: Mixed dot-prefix normalization ==="

echo "Testing mixed normalization: patch000/1.txt ./patch000/2.txt patch000/xxx/3.txt"
"$DAT3" a test7.dat patch000/1.txt ./patch000/2.txt patch000/xxx/3.txt
echo "Mixed normalization archive contents:"
"$DAT3" l test7.dat

# All files should keep their paths
echo "Verifying mixed normalization..."
verify_file_exists "test7.dat" "patch000/1.txt"
verify_file_exists "test7.dat" "patch000/2.txt"
verify_file_exists "test7.dat" "patch000/xxx/3.txt"
echo "Mixed normalization verification passed!"

# Test 8: Glob patterns with ./ prefix keep their directory
echo ""
echo "=== Test 8: Glob patterns with ./ prefix ==="

echo "Testing glob with ./ prefix: ./patch000/*.txt ./patch000/data.bin"
"$DAT3" a test8.dat './patch000/*.txt' './patch000/data.bin'
echo "Glob with ./ prefix archive contents:"
"$DAT3" l test8.dat

# Dot prefix should be removed but directory preserved
echo "Verifying glob with ./ prefix..."
verify_file_exists "test8.dat" "patch000/1.txt"
verify_file_exists "test8.dat" "patch000/2.txt"
verify_file_exists "test8.dat" "patch000/data.bin"
echo "Glob with ./ prefix verification passed!"

# Test 9: Glob pattern filtering when listing archive contents
echo ""
echo "=== Test 9: Glob pattern filtering for list command ==="

# Create a test archive with various file types
"$DAT3" a test9.dat patch000/

echo "Testing glob filter: *.txt"
OUTPUT=$("$DAT3" l test9.dat '*.txt')
echo "$OUTPUT"

# Verify .txt files are listed
echo "$OUTPUT" | grep -q "1.txt" || {
	echo "ERROR: 1.txt not found"
	exit 1
}
echo "$OUTPUT" | grep -q "2.txt" || {
	echo "ERROR: 2.txt not found"
	exit 1
}
echo "$OUTPUT" | grep -q "3.txt" || {
	echo "ERROR: 3.txt not found"
	exit 1
}
echo "$OUTPUT" | grep -q "nested.txt" || {
	echo "ERROR: nested.txt not found"
	exit 1
}

# Verify non-.txt files are NOT listed
if echo "$OUTPUT" | grep -q "data.bin"; then
	echo "ERROR: data.bin should not be listed with *.txt filter"
	exit 1
fi
if echo "$OUTPUT" | grep -q "test.dat"; then
	echo "ERROR: test.dat should not be listed with *.txt filter"
	exit 1
fi
echo "Glob filter for list passed!"

# Test 10: Glob pattern with path prefix
echo ""
echo "=== Test 10: Glob pattern with path for list command ==="

OUTPUT=$("$DAT3" l test9.dat 'patch000/xxx/*')
echo "$OUTPUT"

# Should only match files in patch000/xxx/
echo "$OUTPUT" | grep -q "3.txt" || {
	echo "ERROR: xxx/3.txt not found"
	exit 1
}

# Should NOT match files in other directories
if echo "$OUTPUT" | grep -q "1.txt"; then
	echo "ERROR: 1.txt should not match patch000/xxx/*"
	exit 1
fi
echo "Glob filter with path passed!"

# Test 11: Glob pattern filtering for extract command
echo ""
echo "=== Test 11: Glob pattern filtering for extract command ==="

rm -rf extract_test
mkdir extract_test

# Extract only .txt files
"$DAT3" x test9.dat '*.txt' -o extract_test/

# Verify .txt files were extracted
[ -f "extract_test/patch000/1.txt" ] || {
	echo "ERROR: 1.txt not extracted"
	exit 1
}
[ -f "extract_test/patch000/2.txt" ] || {
	echo "ERROR: 2.txt not extracted"
	exit 1
}

# Verify non-.txt files were NOT extracted
if [ -f "extract_test/patch000/data.bin" ]; then
	echo "ERROR: data.bin should not be extracted with *.txt filter"
	exit 1
fi
echo "Glob filter for extract passed!"

# Test 12: Question mark glob pattern for filtering
echo ""
echo "=== Test 12: Question mark glob for filtering ==="

OUTPUT=$("$DAT3" l test9.dat 'patch000/?.txt')
echo "$OUTPUT"

# Should match 1.txt and 2.txt but not nested.txt
echo "$OUTPUT" | grep -q "1.txt" || {
	echo "ERROR: 1.txt not found"
	exit 1
}
echo "$OUTPUT" | grep -q "2.txt" || {
	echo "ERROR: 2.txt not found"
	exit 1
}

if echo "$OUTPUT" | grep -q "nested.txt"; then
	echo "ERROR: nested.txt should not match ?.txt pattern"
	exit 1
fi
echo "Question mark glob filter passed!"

echo ""
echo "All glob tests completed successfully!"
