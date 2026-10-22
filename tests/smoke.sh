#!/bin/bash

set -xeu -o pipefail

# Usage: ./smoke.sh <dat3-binary>
# Smoke-test a build on the platform it targets, for the builds nothing on Linux
# runs natively: add a tree, read it back, delete an entry.

# Absolute, and resolved before any cd, so a path relative to the caller works
BIN="$(cd "$(dirname "${1:?pass the dat3 binary to test}")" && pwd)/$(basename "$1")"

# Work inside tests directory
cd "$(dirname "$0")"

TEST_DIR="test_smoke"
ARCHIVE="test.dat"

rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data/sub"
cd "$TEST_DIR"

echo "hello" >data/a.txt
seq 3000 | awk '{print "compressible line"}' >data/sub/text.txt

# Write an archive and read it back: the tree must come out unchanged
"$BIN" a -c 9 "$ARCHIVE" data
"$BIN" x "$ARCHIVE" -o out
diff -r data out/data

# Deleting rewrites the archive, so it exercises the save path too
"$BIN" d "$ARCHIVE" "data/a.txt"
rm -rf out
"$BIN" x "$ARCHIVE" -o out
if [ -e "out/data/a.txt" ]; then
    echo "Error: deleted file still present in archive"
    exit 1
fi
diff data/sub/text.txt out/data/sub/text.txt

# Clean up
cd ..
rm -rf "$TEST_DIR"

echo "Smoke test passed: $BIN"
