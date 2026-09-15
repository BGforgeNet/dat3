#!/bin/bash

set -xeu -o pipefail

# Smoke-test the macOS universal binary. Nothing on Linux can run it, so CI runs
# this on an Intel and an Apple Silicon runner, each exercising its own slice.
# MACOS_DAT3 is the binary to test.

# Absolute, and resolved before any cd, so a path relative to the caller works
MACOS_BIN="$(cd "$(dirname "${MACOS_DAT3:?set MACOS_DAT3 to the binary to test}")" && pwd)/$(basename "$MACOS_DAT3")"

# Work inside tests directory
cd "$(dirname "$0")"

TEST_DIR="test_macos"
ARCHIVE="test.dat"

# Without a native slice an Apple Silicon runner with Rosetta would run the
# x86_64 one instead and still pass
ARCH="$(uname -m)"
if ! lipo "$MACOS_BIN" -verify_arch "$ARCH"; then
	echo "Error: $MACOS_BIN has no $ARCH slice" >&2
	exit 1
fi

rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data/sub"
cd "$TEST_DIR"

echo "hello macos" >data/a.txt
seq 3000 | awk '{print "compressible line"}' >data/sub/text.txt

# Write an archive and read it back: the tree must come out unchanged
"$MACOS_BIN" a -c 9 "$ARCHIVE" data
"$MACOS_BIN" x "$ARCHIVE" -o out
diff -r data out/data

# Deleting rewrites the archive, so it exercises the save path too
"$MACOS_BIN" d "$ARCHIVE" "data/a.txt"
rm -rf out
"$MACOS_BIN" x "$ARCHIVE" -o out
if [ -e "out/data/a.txt" ]; then
	echo "Error: deleted file still present in archive"
	exit 1
fi
diff data/sub/text.txt out/data/sub/text.txt

# Clean up
cd ..
rm -rf "$TEST_DIR"

echo "macOS test passed"
