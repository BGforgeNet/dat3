#!/bin/bash

set -xeu -o pipefail

# Smoke-test the WebAssembly build under a WASI runtime. wasm32-wasip1 has no
# threads, so it is the one target where rayon's parallel extraction degrades to
# a serial walk.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

require_wasmtime

# Absolute so it still resolves after the cd into TEST_DIR. wasmtime loads the
# module from the host, so it does not have to sit inside the sandboxed dir.
WASM="$PWD/../target/wasm32-wasip1/release/dat3.wasm"
TEST_DIR="test_wasm"
ARCHIVE="test.dat"

# The guest only sees the directories granted here, so the whole test runs
# inside TEST_DIR with the cwd as its single preopen.
dat3_wasm() {
	wasmtime run --dir . "$WASM" "$@"
}

rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/data/sub"
cd "$TEST_DIR"

echo "hello wasm" >data/a.txt
seq 3000 | awk '{print "compressible line"}' >data/sub/text.txt

# Write an archive and read it back: the tree must come out unchanged
dat3_wasm a -c 9 "$ARCHIVE" data
dat3_wasm x "$ARCHIVE" -o out
diff -r data out/data

# The native binary and the wasm module must agree on the same archive
dat3_wasm l "$ARCHIVE" >listing_wasm.txt
"$DAT3" l "$ARCHIVE" >listing_native.txt
diff -u listing_native.txt listing_wasm.txt

# Deleting rewrites the archive, so it exercises the save path too
dat3_wasm d "$ARCHIVE" "data/a.txt"
rm -rf out
dat3_wasm x "$ARCHIVE" -o out
if [ -e "out/data/a.txt" ]; then
	echo "Error: deleted file still present in archive"
	exit 1
fi
diff data/sub/text.txt out/data/sub/text.txt

# Clean up
cd ..
rm -rf "$TEST_DIR"
