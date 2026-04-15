#!/bin/bash

set -xeu -o pipefail

# Test validation for add command - missing paths and empty directories

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

# Create test directory
TEST_DIR="test_add_validation"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
cd "$TEST_DIR"

# Test 1: Missing file should fail and not create archive
if $DAT3 a missing_file.dat nonexistent.txt; then
	echo "Error: Should have failed with missing file"
	exit 1
fi

# Verify archive was not created
if [ -f missing_file.dat ]; then
	echo "Error: Archive should not have been created"
	exit 1
fi

# Test 2: Empty directory should fail and not create archive
mkdir empty_dir
if $DAT3 a empty_dir.dat empty_dir; then
	echo "Error: Should have failed with empty directory"
	exit 1
fi

# Verify archive was not created
if [ -f empty_dir.dat ]; then
	echo "Error: Archive should not have been created for empty directory"
	exit 1
fi

# Test 3: Mix of existing and missing files should fail on first missing file
echo "test content" >existing.txt
if $DAT3 a mixed.dat existing.txt missing.txt; then
	echo "Error: Should have failed with missing file in list"
	exit 1
fi

# Verify archive was not created
if [ -f mixed.dat ]; then
	echo "Error: Archive should not have been created when some files missing"
	exit 1
fi

# Test 4: Valid files should succeed and create archive
if ! $DAT3 a valid.dat existing.txt; then
	echo "Error: Should have succeeded with existing file"
	exit 1
fi

# Verify archive was created
if [ ! -f valid.dat ]; then
	echo "Error: Archive should have been created for valid files"
	exit 1
fi

if ! $DAT3 l valid.dat | grep -q "existing.txt"; then
	echo "Error: Plain add should store relative path for existing.txt"
	exit 1
fi

if $DAT3 l valid.dat | grep -q "test_add_validation/existing.txt"; then
	echo "Error: Plain add should not store canonicalized absolute path"
	exit 1
fi

# Test 5: Adding to existing archive with missing files should fail and not modify archive
original_size=$(stat -c%s valid.dat)
if $DAT3 a valid.dat missing_again.txt; then
	echo "Error: Should have failed when adding missing file to existing archive"
	exit 1
fi

# Verify archive was not modified
new_size=$(stat -c%s valid.dat)
if [ "$original_size" -ne "$new_size" ]; then
	echo "Error: Existing archive should not have been modified"
	exit 1
fi

# Test 6: Adding empty directory to existing archive should fail and not modify archive
if $DAT3 a valid.dat empty_dir; then
	echo "Error: Should have failed when adding empty directory to existing archive"
	exit 1
fi

# Verify archive was not modified again
final_size=$(stat -c%s valid.dat)
if [ "$original_size" -ne "$final_size" ]; then
	echo "Error: Existing archive should not have been modified by empty directory"
	exit 1
fi

# Test 7: -C should make archive paths relative to the provided directory
mkdir -p modroot/patch000
echo "mod content" >modroot/patch000/from_c.txt
if ! $DAT3 a from_c.dat --dat1 -C modroot patch000/from_c.txt; then
	echo "Error: Should have succeeded with -C relative path"
	exit 1
fi

if ! $DAT3 l from_c.dat | grep -q "patch000/from_c.txt"; then
	echo "Error: File added with -C was stored with the wrong archive path"
	exit 1
fi

# Test 8: -C should reject paths that escape the provided directory
echo "outside" >outside.txt
if $DAT3 a escaped.dat --dat1 -C modroot ../outside.txt; then
	echo "Error: Should have failed when path escapes -C directory"
	exit 1
fi

if [ -f escaped.dat ]; then
	echo "Error: Archive should not have been created for -C traversal"
	exit 1
fi

# Test 9: -C should allow absolute paths that stay inside the provided directory
absolute_in_root="$(pwd)/modroot/patch000/from_c.txt"
if ! $DAT3 a from_c_absolute.dat --dat1 -C modroot "$absolute_in_root"; then
	echo "Error: Should have succeeded with absolute path inside -C directory"
	exit 1
fi

if ! $DAT3 l from_c_absolute.dat | grep -q "patch000/from_c.txt"; then
	echo "Error: Absolute path inside -C directory was stored incorrectly"
	exit 1
fi

# Test 10: -C should skip symlinks instead of archiving their targets
mkdir -p modroot_symlink/patch001
echo "real content" >modroot_symlink/patch001/real.txt
echo "secret content" >modroot_symlink/secret.txt
ln -s ../secret.txt modroot_symlink/patch001/link.txt
if ! $DAT3 a symlink_skip.dat --dat1 -C modroot_symlink patch001; then
	echo "Error: Should have succeeded while skipping symlink"
	exit 1
fi

if ! $DAT3 l symlink_skip.dat | grep -q "patch001/real.txt"; then
	echo "Error: Real file missing when adding directory with symlink"
	exit 1
fi

if $DAT3 l symlink_skip.dat | grep -q "patch001/link.txt"; then
	echo "Error: Symlink should have been skipped"
	exit 1
fi

# Clean up
cd ..
rm -rf "$TEST_DIR"
