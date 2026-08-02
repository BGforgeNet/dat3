#!/bin/bash

set -xeu -o pipefail

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

# Cross-check the Arcanum format against Troika's own dbmaker.exe
# (usage: dbmaker [option] output input; -r recurse, -u unpack, -cN level).
# Absolute path: the helper is called from working directories other than
# this script's own, where a bare dbmaker.exe would not resolve.
DBMAKER_EXE="$PWD/dbmaker.exe"
dbmaker() {
	WINEDEBUG=-all wine "$DBMAKER_EXE" "$@" 2>/dev/null
}

SRC_DIR="arcanum_src"
DAT3_DAT="arcanum_dat3.dat"
DB_DAT="arcanum_dbmaker.dat"
DB_OUT="arcanum_db_out"
DAT3_OUT="arcanum_dat3_out"

# Build a small deterministic source tree: compressible files, a subdirectory,
# and a file so small it stores uncompressed (zlib overhead exceeds its size)
rm -rf "$SRC_DIR"
mkdir -p "$SRC_DIR/data/sub"
seq 1 2000 >"$SRC_DIR/data/numbers.txt"
printf 'hi\n' >"$SRC_DIR/data/sub/tiny.txt"
head -c 4096 /dev/zero >"$SRC_DIR/data/sub/zeros.bin"

# dat3 writes, dbmaker reads
rm -f "$DAT3_DAT"
(cd "$SRC_DIR" && $DAT3 a --format arcanum -c 9 "../$DAT3_DAT" data)
rm -rf "$DB_OUT"
mkdir "$DB_OUT"
(cd "$DB_OUT" && dbmaker -u "../$DAT3_DAT")
diff -r "$SRC_DIR" "$DB_OUT"

# dbmaker writes, dat3 auto-detects and reads. dbmaker's input operand takes
# files, not directory names (a directory silently packs nothing), so name
# the tree explicitly, with the format's backslash separators.
rm -f "$DB_DAT"
(cd "$SRC_DIR" && dbmaker -q "../$DB_DAT" 'data\numbers.txt' 'data\sub\tiny.txt' 'data\sub\zeros.bin')
ls -l "$DB_DAT"
$DAT3 l "$DB_DAT"
# 2 header lines + 3 files; a silent empty build must die here, not at diff
[ "$($DAT3 l "$DB_DAT" | wc -l)" -eq 5 ]
rm -rf "$DAT3_OUT"
$DAT3 x "$DB_DAT" -o "$DAT3_OUT"
diff -r "$SRC_DIR" "$DAT3_OUT"

# dat3 modifies the dbmaker-written archive; dbmaker reads the result
echo "added later" >added.txt
$DAT3 a "$DB_DAT" -t data added.txt
$DAT3 d "$DB_DAT" "data/sub/zeros.bin"
rm -rf "$DB_OUT"
mkdir "$DB_OUT"
(cd "$DB_OUT" && dbmaker -u "../$DB_DAT")
diff "$DB_OUT/data/added.txt" added.txt
if [ -e "$DB_OUT/data/sub/zeros.bin" ]; then
	echo "Error: deleted file still present in archive"
	exit 1
fi

# ── Real game data: the Arcanum demo's 30 MB archive (13k files) ──

# Only the extracted archive is cached; the demo installer is fetched and
# unpacked (RAR -> CAB -> DAT) solely to produce it
DEMO_DAT="ArcanumDemo.dat"
if [ ! -f "$DEMO_DAT" ]; then
	DEMO_RAR="ArcanumDemo.rar"
	if [ ! -f "$DEMO_RAR" ]; then
		wget -nv -O "$DEMO_RAR" "https://archive.org/download/ArcanumDemo/ArcanumDemo.rar"
	fi
	echo "4a77a6cf6f801855cdfe0e7c61459a43  $DEMO_RAR" | md5sum -c
	7z e -y "$DEMO_RAR" Setup1.cab
	7z e -y Setup1.cab Arcanum.dat
	mv Arcanum.dat "$DEMO_DAT"
	rm -f Setup1.cab "$DEMO_RAR"
fi
echo "74e813d99a239c6ee04dd9fa807375a3  $DEMO_DAT" | md5sum -c

# Both tools extract the real archive; the trees must be identical
$DAT3 l "$DEMO_DAT"
rm -rf demo_dat3 demo_db
$DAT3 x "$DEMO_DAT" -o demo_dat3
mkdir demo_db
(cd demo_db && dbmaker -u "../$DEMO_DAT")
diff -r demo_dat3 demo_db

# dat3 modifies the real archive; dbmaker reads the result
DEMO_MOD="demo_mod.dat"
cp "$DEMO_DAT" "$DEMO_MOD"
$DAT3 d "$DEMO_MOD" "WorldMap/WorldMap.mes"
echo "demo test file" >demo_add.txt
$DAT3 a "$DEMO_MOD" -t WorldMap demo_add.txt
rm -rf demo_db
mkdir demo_db
(cd demo_db && dbmaker -u "../$DEMO_MOD")
diff demo_db/WorldMap/demo_add.txt demo_add.txt
if [ -e demo_db/WorldMap/WorldMap.mes ]; then
	echo "Error: deleted file still present in archive"
	exit 1
fi

# Clean up (keep the extracted DAT for the CI cache)
rm -rf "$SRC_DIR" "$DB_OUT" "$DAT3_OUT" "$DAT3_DAT" "$DB_DAT" added.txt \
	demo_dat3 demo_db "$DEMO_MOD" demo_add.txt
