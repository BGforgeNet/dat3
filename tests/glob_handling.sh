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

# Test 5: Directory stripping with ./ prefix
echo ""
echo "=== Test 5: Directory stripping with ./ prefix ==="

# Test Linux build
echo "Testing Linux directory stripping: ./patch000/*"
"$DAT3" a test5_linux.dat './patch000/*'
echo "Linux directory stripping archive contents:"
"$DAT3" l test5_linux.dat

# Verify Linux directory stripping - files should be at root level
echo "Verifying Linux directory stripping..."
for file in 1.txt 2.txt data.bin test.dat xxx/3.txt yyy/nested.txt; do
	verify_file_exists "test5_linux.dat" "$file" "linux"
done
echo "Linux directory stripping verification passed!"

# Test Windows build
echo "Testing Windows directory stripping: .\\patch000\\*"
run_wine a test5_windows.dat '.\\patch000\\*'
echo "Windows directory stripping archive contents:"
run_wine l test5_windows.dat

# Verify Windows directory stripping - files should be at root level
echo "Verifying Windows directory stripping..."
for file in 1.txt 2.txt data.bin test.dat xxx\\3.txt yyy\\nested.txt; do
	verify_file_exists "test5_windows.dat" "$file" "windows"
done
echo "Windows directory stripping verification passed!"

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

# Test 7: Mixed directory stripping (some patterns with ./, some without)
echo ""
echo "=== Test 7: Mixed directory stripping behavior ==="

# Test Linux build with mixed patterns
echo "Testing Linux mixed stripping: patch000/1.txt ./patch000/2.txt patch000/xxx/3.txt"
"$DAT3" a test7_linux.dat patch000/1.txt ./patch000/2.txt patch000/xxx/3.txt
echo "Linux mixed stripping archive contents:"
"$DAT3" l test7_linux.dat

# Verify Linux mixed stripping - only files from ./ patterns should be stripped
echo "Verifying Linux mixed stripping..."
# Files from normal patterns should keep their paths
verify_file_exists "test7_linux.dat" "patch000/1.txt" "linux"
verify_file_exists "test7_linux.dat" "patch000/xxx/3.txt" "linux"
# Files from ./ patterns should be stripped
verify_file_exists "test7_linux.dat" "2.txt" "linux"
echo "Linux mixed stripping verification passed!"

# Test Windows build with mixed patterns
printf "Testing Windows mixed stripping: patch000\\1.txt .\\patch000\\2.txt patch000\\xxx\\3.txt\n"
run_wine a test7_windows.dat 'patch000\1.txt' '.\patch000\2.txt' 'patch000\xxx\3.txt'
echo "Windows mixed stripping archive contents:"
run_wine l test7_windows.dat

# Verify Windows mixed stripping - only files from .\ patterns should be stripped
echo "Verifying Windows mixed stripping..."
# Files from normal patterns should keep their paths
verify_file_exists "test7_windows.dat" "patch000\\1.txt" "windows"
verify_file_exists "test7_windows.dat" "patch000\\xxx\\3.txt" "windows"
# Files from .\ patterns should be stripped
verify_file_exists "test7_windows.dat" "2.txt" "windows"
echo "Windows mixed stripping verification passed!"

# Test 8: Glob patterns with ./ prefix (both individual files and globs should be stripped)
echo ""
echo "=== Test 8: Glob patterns with ./ prefix ==="

# Test Linux build - glob pattern with ./ prefix
echo "Testing Linux glob with ./ prefix: ./patch000/*.txt ./patch000/data.bin"
"$DAT3" a test8_linux.dat './patch000/*.txt' './patch000/data.bin'
echo "Linux glob with ./ prefix archive contents:"
"$DAT3" l test8_linux.dat

# Verify Linux - all files from ./ patterns should be stripped
echo "Verifying Linux glob with ./ prefix..."
verify_file_exists "test8_linux.dat" "1.txt" "linux"
verify_file_exists "test8_linux.dat" "2.txt" "linux"
verify_file_exists "test8_linux.dat" "data.bin" "linux"
echo "Linux glob with ./ prefix verification passed!"

# Test Windows build - glob pattern with .\ prefix  
echo "Testing Windows glob with .\\ prefix: .\\patch000\\*.txt .\\patch000\\data.bin"
run_wine a test8_windows.dat '.\\patch000\\*.txt' '.\\patch000\\data.bin'
echo "Windows glob with .\\ prefix archive contents:"
run_wine l test8_windows.dat

# Verify Windows - all files from .\ patterns should be stripped
echo "Verifying Windows glob with .\\ prefix..."
verify_file_exists "test8_windows.dat" "1.txt" "windows"
verify_file_exists "test8_windows.dat" "2.txt" "windows"
verify_file_exists "test8_windows.dat" "data.bin" "windows"
echo "Windows glob with .\\ prefix verification passed!"

echo ""
echo "All glob tests completed successfully!"
echo "Both Linux and Windows builds passed all glob pattern tests!"
