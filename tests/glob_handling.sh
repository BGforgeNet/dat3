#!/bin/bash

set -xeu -o pipefail

# shellcheck source=tests/common.sh
source "$(dirname "$0")/common.sh"

TEST_DIR="test_glob_handling"

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

# Test 5: Mixed file type patterns (multiple patterns)
echo ""
echo "=== Test 5: Mixed file type glob patterns ==="

# Test Linux build with multiple patterns
echo "Testing Linux mixed file types: patch000/*.txt patch000/*.dat patch000/*.bin"
"$DAT3" a test5_linux.dat 'patch000/*.txt' 'patch000/*.dat' 'patch000/*.bin'
echo "Linux mixed file type glob archive contents:"
"$DAT3" l test5_linux.dat

# Verify Linux mixed file types
echo "Verifying Linux mixed file types..."
for file in patch000/1.txt patch000/2.txt patch000/data.bin patch000/test.dat; do
	verify_file_exists "test5_linux.dat" "$file" "linux"
done
echo "Linux mixed file type glob pattern verification passed!"

# Test Windows build with multiple patterns
echo "Testing Windows mixed file types: patch000\\*.txt patch000\\*.dat patch000\\*.bin"
run_wine a test5_windows.dat 'patch000\*.txt' 'patch000\*.dat' 'patch000\*.bin'
echo "Windows mixed file type glob archive contents:"
run_wine l test5_windows.dat

# Verify Windows mixed file types
echo "Verifying Windows mixed file types..."
for file in patch000\\1.txt patch000\\2.txt patch000\\data.bin patch000\\test.dat; do
	verify_file_exists "test5_windows.dat" "$file" "windows"
done
echo "Windows mixed file type glob pattern verification passed!"

echo ""
echo "All glob tests completed successfully!"
echo "Both Linux and Windows builds passed all glob pattern tests!"
