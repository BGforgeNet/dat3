#!/bin/bash

set -xeu -o pipefail

# dat3 against the Arcanum format: round-trip a small tree, then list,
# extract, repack and modify the demo's real 30 MB archive.
# The dbmaker.exe cross-checks live in arcanum_wine.sh.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

SRC_DIR="arcanum_src"
DAT3_DAT="arcanum_dat3.dat"
DAT3_OUT="arcanum_dat3_out"

build_arcanum_src_tree "$SRC_DIR"

# Write an archive and read it back: the tree must come out unchanged
rm -f "$DAT3_DAT"
(cd "$SRC_DIR" && $DAT3 a --format arcanum -c 9 "../$DAT3_DAT" data)
rm -rf "$DAT3_OUT"
$DAT3 x "$DAT3_DAT" -o "$DAT3_OUT"
diff -r "$SRC_DIR" "$DAT3_OUT"

# Delete and add against that archive
echo "added later" >added.txt
$DAT3 a "$DAT3_DAT" -t data added.txt
$DAT3 d "$DAT3_DAT" "data/sub/zeros.bin"
rm -rf "$DAT3_OUT"
$DAT3 x "$DAT3_DAT" -o "$DAT3_OUT"
diff "$DAT3_OUT/data/added.txt" added.txt
if [ -e "$DAT3_OUT/data/sub/zeros.bin" ]; then
	echo "Error: deleted file still present in archive"
	exit 1
fi

# -- Real game data: the Arcanum demo's 30 MB archive (13k files) --

fetch_arcanum_demo

DEMO_OUT="demo_dat3"
DEMO_REPACKED="demo_repacked.dat"
DEMO_REPACKED_OUT="demo_repacked_out"

$DAT3 l "$ARCANUM_DEMO_DAT"
rm -rf "$DEMO_OUT"
$DAT3 x "$ARCANUM_DEMO_DAT" -o "$DEMO_OUT"

# Repack the extracted tree and extract it again: 13k files must round-trip
rm -f "$DEMO_REPACKED"
(cd "$DEMO_OUT" && $DAT3 a --format arcanum -c 9 "../$DEMO_REPACKED" .)
rm -rf "$DEMO_REPACKED_OUT"
$DAT3 x "$DEMO_REPACKED" -o "$DEMO_REPACKED_OUT"
diff -r "$DEMO_OUT" "$DEMO_REPACKED_OUT"

# dat3 modifies the real archive
DEMO_MOD="demo_mod.dat"
MOD_OUT="demo_mod_out"
cp "$ARCANUM_DEMO_DAT" "$DEMO_MOD"
$DAT3 d "$DEMO_MOD" "WorldMap/WorldMap.mes"
echo "demo test file" >demo_add.txt
$DAT3 a "$DEMO_MOD" -t WorldMap demo_add.txt
rm -rf "$MOD_OUT"
$DAT3 x "$DEMO_MOD" -o "$MOD_OUT"
diff "$MOD_OUT/WorldMap/demo_add.txt" demo_add.txt
if [ -e "$MOD_OUT/WorldMap/WorldMap.mes" ]; then
	echo "Error: deleted file still present in archive"
	exit 1
fi

# The deleted name must now be rejected rather than silently ignored
if $DAT3 l "$DEMO_MOD" "WorldMap/WorldMap.mes" 2>/dev/null; then
	echo "Error: l should fail for a file that is not in the archive"
	exit 1
fi

# Clean up (keep the extracted DAT for the CI cache)
rm -rf "$SRC_DIR" "$DAT3_OUT" "$DAT3_DAT" added.txt \
	"$DEMO_OUT" "$DEMO_REPACKED" "$DEMO_REPACKED_OUT" "$DEMO_MOD" "$MOD_OUT" demo_add.txt
