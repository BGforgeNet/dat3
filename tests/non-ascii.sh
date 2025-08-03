#!/bin/bash

set -xeu -o pipefail

# Test that dat3 properly fails when encountering non-ASCII filenames

# Load common variables and functions
source "$(dirname "$0")/common.sh"

# Create a temporary directory for our test
TEST_DIR="test_non_ascii_$$"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Test 1: ASCII filename should work
echo "test content" >"test.txt"
$DAT3 a test.dat "test.txt" >/dev/null 2>&1
rm -f test.dat

# Test 2: Non-ASCII filename should fail
echo "test content" >"tëst.txt"
if $DAT3 a test.dat "tëst.txt" >/dev/null 2>&1; then
	exit 1
fi

# Test 3: Directory with non-ASCII filename should fail
mkdir -p "tëst_dir"
echo "test content" >"tëst_dir/file.txt"
if $DAT3 a test2.dat "tëst_dir" >/dev/null 2>&1; then
	exit 1
fi

# Clean up
cd ..
rm -rf "$TEST_DIR"
