#!/bin/bash

set -xeu -o pipefail

# shellcheck source=tests/common.sh
source "$(dirname "$0")/common.sh"

TEST_DIR="test_path_consistency"

# Clean up any previous test
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

echo "Testing path consistency with wildcard expansion..."

# Create test directory structure like the user's case
mkdir -p patch000/xxx
echo "content1" >patch000/1.txt
echo "content2" >patch000/2.txt
echo "content3" >patch000/xxx/3.txt

# Test the issue: dat3 a patch000.dat patch000/*
echo "Creating archive with patch000/* expansion..."
"$DAT3" a patch000.dat patch000/* -c9

echo "Listing archive contents..."
"$DAT3" l patch000.dat

# Check for path consistency - look for paths that don't start with patch000/
echo "Checking path consistency..."
if "$DAT3" l patch000.dat | awk 'NR>2 {print $4}' | grep -v "^patch000/"; then
	echo "ERROR: Found paths that don't start with 'patch000/'"
	exit 1
fi

# Verify all paths start with patch000/
count=$("$DAT3" l patch000.dat | grep -c "patch000/" || true)
if [ "$count" -ne 3 ]; then
	echo "ERROR: Expected 3 files with patch000/ prefix, found $count"
	"$DAT3" l patch000.dat
	exit 1
fi

echo "Path consistency test passed!"

# Clean up
cd ..
rm -rf "$TEST_DIR"
