#!/bin/bash

set -xeu -o pipefail

# Smoke-test the Linux arm64 build under qemu. It is the only released binary
# whose allocator is built by a different C compiler (zig, see build.sh), so it
# gets exercised rather than only linked. Needs ./build.sh to have run.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

require_qemu_aarch64

# Absolute so it still resolves after the cd into TEST_DIR
ARM_DAT3="$PWD/../target/aarch64-unknown-linux-musl/release/dat3"
TEST_DIR="test_arm64"
ARCHIVE="test.dat"

dat3_arm64() {
	qemu-aarch64-static "$ARM_DAT3" "$@"
}

rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data/sub"
cd "$TEST_DIR"

echo "hello arm64" >data/a.txt
seq 3000 | awk '{print "compressible line"}' >data/sub/text.txt

# Write an archive and read it back: the tree must come out unchanged
dat3_arm64 a -c 9 "$ARCHIVE" data
dat3_arm64 x "$ARCHIVE" -o out
diff -r data out/data

# The native binary and the arm64 one must agree on the same archive
dat3_arm64 l "$ARCHIVE" >listing_arm64.txt
"$DAT3" l "$ARCHIVE" >listing_native.txt
diff -u listing_native.txt listing_arm64.txt

# Deleting rewrites the archive, so it exercises the save path too
dat3_arm64 d "$ARCHIVE" "data/a.txt"
rm -rf out
dat3_arm64 x "$ARCHIVE" -o out
if [ -e "out/data/a.txt" ]; then
	echo "Error: deleted file still present in archive"
	exit 1
fi
diff data/sub/text.txt out/data/sub/text.txt

# Clean up
cd ..
rm -rf "$TEST_DIR"
