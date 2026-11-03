#!/bin/bash

set -xeu -o pipefail

# --dedup on a and d: identical files are stored once in every format, and
# every entry still extracts as added.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

TEST_DIR="$(pwd)/test_dedup"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
# Removed on exit, so a failing check leaves nothing behind either
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$TEST_DIR"

mkdir -p src/art/sub
seq 1 5000 >src/art/a.frm
cp src/art/a.frm src/art/sub/b.frm
echo "other" >src/c.txt

size() {
    wc -c <"$1" | tr -d ' '
}

for format in dat1 dat2 arcanum toee; do
    $DAT3 a --format "$format" -C src "full_$format.dat" art c.txt
    $DAT3 a --format "$format" --dedup -C src "dedup_$format.dat" art c.txt

    full=$(size "full_$format.dat")
    dedup=$(size "dedup_$format.dat")
    if [ "$dedup" -ge "$full" ]; then
        echo "Error: $format: --dedup archive ($dedup bytes) is not smaller than the plain one ($full bytes)"
        exit 1
    fi

    $DAT3 x -o "out_$format" "dedup_$format.dat"
    diff -r src "out_$format"

    # Deleting the entry whose data was written first keeps its sharer intact
    cp "dedup_$format.dat" "deleted_$format.dat"
    $DAT3 d --dedup "deleted_$format.dat" art/a.frm
    $DAT3 x -o "deleted_out_$format" "deleted_$format.dat"
    cmp src/art/sub/b.frm "deleted_out_$format/art/sub/b.frm"
done

echo "All dedup tests passed"
