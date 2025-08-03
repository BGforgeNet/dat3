#!/bin/bash

set -xeu -o pipefail

# Test response file functionality with a dedicated test archive

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

# Create test directory
TEST_DIR="test_response_file"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Create test files for the archive
mkdir -p test_data/dir1/subdir
mkdir -p test_data/dir2

echo "File 1 content" >test_data/file1.txt
echo "File 2 content" >test_data/dir1/file2.txt
echo "File 3 content" >test_data/dir1/subdir/file3.txt
echo "File 4 content" >test_data/dir2/file4.txt
echo "File 5 content" >test_data/file5.txt

# Create a test archive
$DAT3 a test_response.dat test_data

# Create response file with specific files to extract
cat >test_response.txt <<EOF
test_data/file1.txt
test_data/dir1/file2.txt
test_data/dir1/subdir/file3.txt
EOF

# Test list command with response file
$DAT3 l test_response.dat @test_response.txt

# Test extraction with response file
rm -rf response_test
mkdir response_test
$DAT3 x test_response.dat -o response_test @test_response.txt

# Verify only the three files in response file were extracted
verify_file "response_test/test_data/file1.txt"
verify_file "response_test/test_data/dir1/file2.txt"
verify_file "response_test/test_data/dir1/subdir/file3.txt"

# Verify files NOT in response file were NOT extracted
if [ -f "response_test/test_data/dir2/file4.txt" ]; then
	echo "Error: file4.txt should not have been extracted"
	exit 1
fi
if [ -f "response_test/test_data/file5.txt" ]; then
	echo "Error: file5.txt should not have been extracted"
	exit 1
fi

# Test flat extraction with response file
rm -rf response_flat
mkdir response_flat
$DAT3 e test_response.dat -o response_flat @test_response.txt

# Verify all three files were extracted to flat directory
verify_file "response_flat/file1.txt"
verify_file "response_flat/file2.txt"
verify_file "response_flat/file3.txt"

# Test error case - mixing @response-file with explicit files
if $DAT3 l test_response.dat @test_response.txt test_data/file5.txt 2>/dev/null; then
	echo "Error: Command should have failed when mixing @response-file with explicit files"
	exit 1
fi

# Test response file with mixed path separators
cat >test_response_mixed.txt <<EOF
test_data/file1.txt
test_data\dir1\file2.txt
test_data/dir1\subdir/file3.txt
EOF

# Should work with mixed separators
$DAT3 l test_response.dat @test_response_mixed.txt

# Clean up
cd ..
rm -rf "$TEST_DIR"
