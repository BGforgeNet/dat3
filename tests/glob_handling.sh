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

# Get the Windows binary path for all tests
WIN_BINARY="$SCRIPT_DIR/../target/x86_64-pc-windows-gnu/release/dat3.exe"
WINE_DAT3="$(winepath -w "$WIN_BINARY")"

echo ""
echo "=== Test 1: Basic glob pattern ==="

# Test Linux build
echo "Testing Linux: dat3 a test1_linux.dat 'patch000/*.txt'"
"$DAT3" a test1_linux.dat 'patch000/*.txt'
echo "Linux archive contents:"
"$DAT3" l test1_linux.dat

# Verify Linux glob expansion
echo "Verifying Linux glob expansion..."
if ! "$DAT3" l test1_linux.dat patch000/1.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - patch000/1.txt not found in archive"
	exit 1
fi
if ! "$DAT3" l test1_linux.dat patch000/2.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - patch000/2.txt not found in archive"
	exit 1
fi
# Should NOT contain xxx/3.txt since it's in a subdirectory
if "$DAT3" l test1_linux.dat patch000/xxx/3.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - patch000/xxx/3.txt should not be in archive (subdirectory)"
	exit 1
fi
echo "Linux basic glob pattern verification passed!"

# Test Windows build
echo "Testing Windows: wine cmd /c \"$WINE_DAT3 a test1_windows.dat patch000\\*.txt\""
wine cmd /c "$WINE_DAT3 a test1_windows.dat patch000\\*.txt"
echo "Windows archive contents:"
wine cmd /c "$WINE_DAT3 l test1_windows.dat"

# Verify Windows glob expansion
echo "Verifying Windows glob expansion..."
if ! wine cmd /c "$WINE_DAT3 l test1_windows.dat patch000\\1.txt" >/dev/null 2>&1; then
	echo "ERROR: Windows - glob expansion failed"
	exit 1
fi
if ! wine cmd /c "$WINE_DAT3 l test1_windows.dat patch000\\2.txt" >/dev/null 2>&1; then
	printf "ERROR: Windows - patch000\\2.txt not found in archive\n"
	exit 1
fi
echo "Windows basic glob pattern verification passed!"

echo ""
echo "=== Test 2: Recursive glob pattern ==="

# Test Linux build
echo "Testing Linux recursive glob: patch000/**/*.txt"
"$DAT3" a test2_linux.dat 'patch000/**/*.txt'
echo "Linux recursive glob archive contents:"
"$DAT3" l test2_linux.dat

# Verify Linux recursive pattern includes subdirectory files
if ! "$DAT3" l test2_linux.dat patch000/xxx/3.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - Recursive glob should include patch000/xxx/3.txt"
	exit 1
fi
if ! "$DAT3" l test2_linux.dat patch000/yyy/nested.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - Recursive glob should include patch000/yyy/nested.txt"
	exit 1
fi
echo "Linux recursive glob pattern verification passed!"

# Test Windows build
echo "Testing Windows recursive glob: patch000\\**\\*.txt"
wine cmd /c "$WINE_DAT3 a test2_windows.dat patch000\\**\\*.txt"
echo "Windows recursive glob archive contents:"
wine cmd /c "$WINE_DAT3 l test2_windows.dat"

# Verify Windows recursive pattern
if ! wine cmd /c "$WINE_DAT3 l test2_windows.dat patch000\\xxx\\3.txt" >/dev/null 2>&1; then
	printf "ERROR: Windows - Recursive glob should include patch000\\xxx\\3.txt\n"
	exit 1
fi
echo "Windows recursive glob pattern verification passed!"

echo ""
echo "=== Test 3: Character range glob pattern ==="

# Test Linux build
echo "Testing Linux character range glob: patch000/[12].txt"
"$DAT3" a test3_linux.dat 'patch000/[12].txt'
echo "Linux character range glob archive contents:"
"$DAT3" l test3_linux.dat

# Verify Linux character range pattern
if ! "$DAT3" l test3_linux.dat patch000/1.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - Character range glob should include patch000/1.txt"
	exit 1
fi
if ! "$DAT3" l test3_linux.dat patch000/2.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - Character range glob should include patch000/2.txt"
	exit 1
fi
echo "Linux character range glob pattern verification passed!"

# Test Windows build
echo "Testing Windows character range glob: patch000\\[12].txt"
wine cmd /c "$WINE_DAT3 a test3_windows.dat patch000\\[12].txt"
echo "Windows character range glob archive contents:"
wine cmd /c "$WINE_DAT3 l test3_windows.dat"

# Verify Windows character range pattern
if ! wine cmd /c "$WINE_DAT3 l test3_windows.dat patch000\\1.txt" >/dev/null 2>&1; then
	printf "ERROR: Windows - Character range glob should include patch000\\1.txt\n"
	exit 1
fi
echo "Windows character range glob pattern verification passed!"

echo ""
echo "=== Test 4: Question mark glob pattern ==="

# Test Linux build
echo "Testing Linux question mark glob: patch000/?.txt"
"$DAT3" a test4_linux.dat 'patch000/?.txt'
echo "Linux question mark glob archive contents:"
"$DAT3" l test4_linux.dat

# Verify Linux question mark pattern matches single characters
if ! "$DAT3" l test4_linux.dat patch000/1.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - Question mark glob should include patch000/1.txt"
	exit 1
fi
if ! "$DAT3" l test4_linux.dat patch000/2.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - Question mark glob should include patch000/2.txt"
	exit 1
fi
echo "Linux question mark glob pattern verification passed!"

# Test Windows build
echo "Testing Windows question mark glob: patch000\\?.txt"
wine cmd /c "$WINE_DAT3 a test4_windows.dat patch000\\?.txt"
echo "Windows question mark glob archive contents:"
wine cmd /c "$WINE_DAT3 l test4_windows.dat"

# Verify Windows question mark pattern
if ! wine cmd /c "$WINE_DAT3 l test4_windows.dat patch000\\1.txt" >/dev/null 2>&1; then
	printf "ERROR: Windows - Question mark glob should include patch000\\1.txt\n"
	exit 1
fi
echo "Windows question mark glob pattern verification passed!"

echo ""
echo "=== Test 5: Mixed file type glob patterns ==="

# Test Linux build
echo "Testing Linux mixed file types: patch000/*.txt patch000/*.dat patch000/*.bin"
"$DAT3" a test5_linux.dat 'patch000/*.txt' 'patch000/*.dat' 'patch000/*.bin'
echo "Linux mixed file type glob archive contents:"
"$DAT3" l test5_linux.dat

# Verify Linux mixed file types
if ! "$DAT3" l test5_linux.dat patch000/1.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - Mixed types should include patch000/1.txt"
	exit 1
fi
if ! "$DAT3" l test5_linux.dat patch000/2.txt >/dev/null 2>&1; then
	echo "ERROR: Linux - Mixed types should include patch000/2.txt"
	exit 1
fi
if ! "$DAT3" l test5_linux.dat patch000/data.bin >/dev/null 2>&1; then
	echo "ERROR: Linux - Mixed types should include patch000/data.bin"
	exit 1
fi
if ! "$DAT3" l test5_linux.dat patch000/test.dat >/dev/null 2>&1; then
	echo "ERROR: Linux - Mixed types should include patch000/test.dat"
	exit 1
fi
echo "Linux mixed file type glob pattern verification passed!"

# Test Windows build
echo "Testing Windows mixed file types: patch000\\*.txt patch000\\*.dat patch000\\*.bin"
wine cmd /c "$WINE_DAT3 a test5_windows.dat patch000\\*.txt patch000\\*.dat patch000\\*.bin"
echo "Windows mixed file type glob archive contents:"
wine cmd /c "$WINE_DAT3 l test5_windows.dat"

# Verify Windows mixed file types
if ! wine cmd /c "$WINE_DAT3 l test5_windows.dat patch000\\1.txt" >/dev/null 2>&1; then
	printf "ERROR: Windows - Mixed types should include patch000\\1.txt\n"
	exit 1
fi
if ! wine cmd /c "$WINE_DAT3 l test5_windows.dat patch000\\2.txt" >/dev/null 2>&1; then
	printf "ERROR: Windows - Mixed types should include patch000\\2.txt\n"
	exit 1
fi
if ! wine cmd /c "$WINE_DAT3 l test5_windows.dat patch000\\data.bin" >/dev/null 2>&1; then
	printf "ERROR: Windows - Mixed types should include patch000\\data.bin\n"
	exit 1
fi
if ! wine cmd /c "$WINE_DAT3 l test5_windows.dat patch000\\test.dat" >/dev/null 2>&1; then
	printf "ERROR: Windows - Mixed types should include patch000\\test.dat\n"
	exit 1
fi
echo "Windows mixed file type glob pattern verification passed!"

echo ""
echo "All glob tests completed successfully!"
echo "Both Linux and Windows builds passed all glob pattern tests!"
