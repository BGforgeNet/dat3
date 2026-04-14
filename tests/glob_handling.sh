#!/bin/bash

set -xeu -o pipefail

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

# Copy Windows binary to test directory for Wine (like rpu.sh does)
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
	local platform="$3"

	if [ "$platform" = "linux" ]; then
		if ! "$DAT3" l "$archive" "$file" >/dev/null 2>&1; then
			echo "ERROR: Linux - $file not found in archive"
			exit 1
		fi
	else
		if ! run_wine l "$archive" "$file" >/dev/null 2>&1; then
			printf "ERROR: Windows - %s not found in archive\\n" "$file"
			exit 1
		fi
	fi
}

# Helper function to verify file does NOT exist in archive
verify_file_missing() {
	local archive="$1"
	local file="$2"
	local platform="$3"

	if [ "$platform" = "linux" ]; then
		if "$DAT3" l "$archive" "$file" >/dev/null 2>&1; then
			echo "ERROR: Linux - $file should not be in archive"
			exit 1
		fi
	else
		if run_wine l "$archive" "$file" >/dev/null 2>&1; then
			printf "ERROR: Windows - %s should not be in archive\\n" "$file"
			exit 1
		fi
	fi
}

# Test function for a glob pattern on both platforms
test_glob_pattern() {
	local test_num="$1"
	local test_name="$2"
	local linux_pattern="$3"
	local windows_pattern="$4"
	local verify_files="$5"   # space-separated list of files that should exist
	local verify_missing="$6" # space-separated list of files that should NOT exist

	echo ""
	echo "=== Test $test_num: $test_name ==="

	# Test Linux build
	echo "Testing Linux $test_name: $linux_pattern"
	"$DAT3" a "test${test_num}_linux.dat" "$linux_pattern"
	echo "Linux $test_name archive contents:"
	"$DAT3" l "test${test_num}_linux.dat"

	# Verify Linux files
	echo "Verifying Linux $test_name..."
	for file in $verify_files; do
		verify_file_exists "test${test_num}_linux.dat" "$file" "linux"
	done
	for file in $verify_missing; do
		verify_file_missing "test${test_num}_linux.dat" "$file" "linux"
	done
	echo "Linux $test_name verification passed!"

	# Test Windows build
	echo "Testing Windows $test_name: $windows_pattern"
	run_wine a "test${test_num}_windows.dat" "$windows_pattern"
	echo "Windows $test_name archive contents:"
	run_wine l "test${test_num}_windows.dat"

	# Verify Windows files (convert forward slashes to backslashes)
	echo "Verifying Windows $test_name..."
	for file in $verify_files; do
		win_file=${file//\/\\/}
		verify_file_exists "test${test_num}_windows.dat" "$win_file" "windows"
	done
	for file in $verify_missing; do
		win_file=${file//\/\\/}
		verify_file_missing "test${test_num}_windows.dat" "$win_file" "windows"
	done
	echo "Windows $test_name verification passed!"
}

# Run all glob pattern tests
test_glob_pattern "1" "Basic glob pattern" \
	"patch000/*.txt" \
	"patch000\\*.txt" \
	"patch000/1.txt patch000/2.txt" \
	"patch000/xxx/3.txt"

test_glob_pattern "2" "Recursive glob pattern" \
	"patch000/**/*.txt" \
	"patch000\\**\\*.txt" \
	"patch000/1.txt patch000/2.txt patch000/xxx/3.txt patch000/yyy/nested.txt" \
	""

test_glob_pattern "3" "Character range glob pattern" \
	"patch000/[12].txt" \
	"patch000\\[12].txt" \
	"patch000/1.txt patch000/2.txt" \
	""

test_glob_pattern "4" "Question mark glob pattern" \
	"patch000/?.txt" \
	"patch000\\?.txt" \
	"patch000/1.txt patch000/2.txt" \
	""

# Test 5: Dot-prefix normalization with ./ prefix
echo ""
echo "=== Test 5: Dot-prefix normalization with ./ prefix ==="

# Test Linux build
echo "Testing Linux dot-prefix normalization: ./patch000/*"
"$DAT3" a test5_linux.dat './patch000/*'
echo "Linux dot-prefix normalization archive contents:"
"$DAT3" l test5_linux.dat

# Verify Linux dot-prefix normalization - files should keep patch000/ prefix
echo "Verifying Linux dot-prefix normalization..."
for file in patch000/1.txt patch000/2.txt patch000/data.bin patch000/test.dat patch000/xxx/3.txt patch000/yyy/nested.txt; do
	verify_file_exists "test5_linux.dat" "$file" "linux"
done
echo "Linux dot-prefix normalization verification passed!"

# Test Windows build
echo "Testing Windows dot-prefix normalization: .\\patch000\\*"
run_wine a test5_windows.dat '.\\patch000\\*'
echo "Windows dot-prefix normalization archive contents:"
run_wine l test5_windows.dat

# Verify Windows dot-prefix normalization - files should keep patch000\ prefix
echo "Verifying Windows dot-prefix normalization..."
for file in patch000\\1.txt patch000\\2.txt patch000\\data.bin patch000\\test.dat patch000\\xxx\\3.txt patch000\\yyy\\nested.txt; do
	verify_file_exists "test5_windows.dat" "$file" "windows"
done
echo "Windows dot-prefix normalization verification passed!"

# Test 6: Mixed file type patterns (multiple patterns)
echo ""
echo "=== Test 6: Mixed file type glob patterns ==="

# Test Linux build with multiple patterns
echo "Testing Linux mixed file types: patch000/*.txt patch000/*.dat patch000/*.bin"
"$DAT3" a test6_linux.dat 'patch000/*.txt' 'patch000/*.dat' 'patch000/*.bin'
echo "Linux mixed file type glob archive contents:"
"$DAT3" l test6_linux.dat

# Verify Linux mixed file types
echo "Verifying Linux mixed file types..."
for file in patch000/1.txt patch000/2.txt patch000/data.bin patch000/test.dat; do
	verify_file_exists "test6_linux.dat" "$file" "linux"
done
echo "Linux mixed file type glob pattern verification passed!"

# Test Windows build with multiple patterns
echo "Testing Windows mixed file types: patch000\\*.txt patch000\\*.dat patch000\\*.bin"
run_wine a test6_windows.dat 'patch000\*.txt' 'patch000\*.dat' 'patch000\*.bin'
echo "Windows mixed file type glob archive contents:"
run_wine l test6_windows.dat

# Verify Windows mixed file types
echo "Verifying Windows mixed file types..."
for file in patch000\\1.txt patch000\\2.txt patch000\\data.bin patch000\\test.dat; do
	verify_file_exists "test6_windows.dat" "$file" "windows"
done
echo "Windows mixed file type glob pattern verification passed!"

# Test 7: Mixed dot-prefix normalization
echo ""
echo "=== Test 7: Mixed dot-prefix normalization ==="

# Test Linux build with mixed patterns
echo "Testing Linux mixed normalization: patch000/1.txt ./patch000/2.txt patch000/xxx/3.txt"
"$DAT3" a test7_linux.dat patch000/1.txt ./patch000/2.txt patch000/xxx/3.txt
echo "Linux mixed normalization archive contents:"
"$DAT3" l test7_linux.dat

# Verify Linux mixed normalization - all files should keep their paths
echo "Verifying Linux mixed normalization..."
verify_file_exists "test7_linux.dat" "patch000/1.txt" "linux"
verify_file_exists "test7_linux.dat" "patch000/2.txt" "linux"
verify_file_exists "test7_linux.dat" "patch000/xxx/3.txt" "linux"
echo "Linux mixed normalization verification passed!"

# Test Windows build with mixed patterns
printf '%s\n' 'Testing Windows mixed normalization: patch000\1.txt .\patch000\2.txt patch000\xxx\3.txt'
run_wine a test7_windows.dat 'patch000\1.txt' '.\patch000\2.txt' 'patch000\xxx\3.txt'
echo "Windows mixed normalization archive contents:"
run_wine l test7_windows.dat

# Verify Windows mixed normalization - all files should keep their paths
echo "Verifying Windows mixed normalization..."
verify_file_exists "test7_windows.dat" "patch000\\1.txt" "windows"
verify_file_exists "test7_windows.dat" "patch000\\2.txt" "windows"
verify_file_exists "test7_windows.dat" "patch000\\xxx\\3.txt" "windows"
echo "Windows mixed normalization verification passed!"

# Test 8: Glob patterns with ./ prefix keep their directory
echo ""
echo "=== Test 8: Glob patterns with ./ prefix ==="

# Test Linux build - glob pattern with ./ prefix
echo "Testing Linux glob with ./ prefix: ./patch000/*.txt ./patch000/data.bin"
"$DAT3" a test8_linux.dat './patch000/*.txt' './patch000/data.bin'
echo "Linux glob with ./ prefix archive contents:"
"$DAT3" l test8_linux.dat

# Verify Linux - dot prefix should be removed but directory preserved
echo "Verifying Linux glob with ./ prefix..."
verify_file_exists "test8_linux.dat" "patch000/1.txt" "linux"
verify_file_exists "test8_linux.dat" "patch000/2.txt" "linux"
verify_file_exists "test8_linux.dat" "patch000/data.bin" "linux"
echo "Linux glob with ./ prefix verification passed!"

# Test Windows build - glob pattern with .\ prefix
echo "Testing Windows glob with .\\ prefix: .\\patch000\\*.txt .\\patch000\\data.bin"
run_wine a test8_windows.dat '.\\patch000\\*.txt' '.\\patch000\\data.bin'
echo "Windows glob with .\\ prefix archive contents:"
run_wine l test8_windows.dat

# Verify Windows - dot prefix should be removed but directory preserved
echo "Verifying Windows glob with .\\ prefix..."
verify_file_exists "test8_windows.dat" "patch000\\1.txt" "windows"
verify_file_exists "test8_windows.dat" "patch000\\2.txt" "windows"
verify_file_exists "test8_windows.dat" "patch000\\data.bin" "windows"
echo "Windows glob with .\\ prefix verification passed!"

# Test 9: Glob pattern filtering when listing archive contents
echo ""
echo "=== Test 9: Glob pattern filtering for list command ==="

# Create a test archive with various file types
"$DAT3" a test9.dat patch000/

# Test Linux - list only .txt files using glob
echo "Testing Linux glob filter: *.txt"
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
echo "Linux glob filter for list passed!"

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
echo "Linux glob filter with path passed!"

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
echo "Linux glob filter for extract passed!"

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
echo "Both Linux and Windows builds passed all glob pattern tests!"
