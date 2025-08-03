#!/bin/bash

set -xeu -o pipefail

# Test that duplicate file paths are prevented in DAT archives

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

# Create test directory
TEST_DIR="test_duplicate_paths"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Test 1: Adding same file multiple times should not create duplicates
echo "test content" >single_file.txt
$DAT3 a single.dat single_file.txt
$DAT3 a single.dat single_file.txt
$DAT3 a single.dat single_file.txt

# Count occurrences - should be exactly 1
count=$($DAT3 l single.dat | grep -c "single_file.txt" || true)
if [ "$count" -ne 1 ]; then
	echo "Error: Expected 1 occurrence of single_file.txt, got $count"
	$DAT3 l single.dat
	exit 1
fi

# Test 2: Adding same directory multiple times should not create duplicates
mkdir -p testdir
echo "dir content" >testdir/file.txt
$DAT3 a dir.dat testdir
$DAT3 a dir.dat testdir
$DAT3 a dir.dat testdir

# Count occurrences - should be exactly 1
count=$($DAT3 l dir.dat | grep -c "testdir/file.txt" || true)
if [ "$count" -ne 1 ]; then
	echo "Error: Expected 1 occurrence of testdir/file.txt, got $count"
	$DAT3 l dir.dat
	exit 1
fi

# Test 3: Adding multiple files with same filename but different paths (should keep both)
mkdir -p subdir1 subdir2
echo "content1" >subdir1/same.txt
echo "content2" >subdir2/same.txt
$DAT3 a multi.dat subdir1/same.txt subdir2/same.txt

# Should have both files since they're in different directories
count=$($DAT3 l multi.dat | grep -c "same.txt" || true)
if [ "$count" -ne 2 ]; then
	echo "Error: Expected 2 occurrences of same.txt in different directories, got $count"
	$DAT3 l multi.dat
	exit 1
fi

# Test 4: DAT1 format should also prevent duplicates
echo "dat1 content" >dat1_file.txt
$DAT3 a --dat1 dat1.dat dat1_file.txt
$DAT3 a dat1.dat dat1_file.txt
$DAT3 a dat1.dat dat1_file.txt

# Count occurrences - should be exactly 1
count=$($DAT3 l dat1.dat | grep -c "dat1_file.txt" || true)
if [ "$count" -ne 1 ]; then
	echo "Error: Expected 1 occurrence of dat1_file.txt in DAT1 format, got $count"
	$DAT3 l dat1.dat
	exit 1
fi

# Test 5: Mixed paths (forward/back slashes) should be treated as same file
mkdir -p deep/nested
echo "nested content" >deep/nested/file.txt
$DAT3 a mixed.dat deep/nested/file.txt

# Add same file again - should deduplicate
$DAT3 a mixed.dat deep/nested/file.txt

# Check for the file using a pattern that matches the display format
# On Unix: deep/nested/file.txt, On Windows: deep\nested\file.txt
count=$($DAT3 l mixed.dat | grep -c "nested.*file.txt" || true)
if [ "$count" -ne 1 ]; then
	echo "Error: Expected 1 occurrence with mixed path separators, got $count"
	$DAT3 l mixed.dat
	exit 1
fi

echo "All duplicate path tests passed"

# Clean up
cd ..
rm -rf "$TEST_DIR"
