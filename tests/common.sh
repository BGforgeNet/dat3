#!/bin/bash

set -xeu -o pipefail

# Common variables and functions for test scripts

# Use static build - get absolute path relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DAT3="$SCRIPT_DIR/../target/x86_64-unknown-linux-musl/release/dat3"
export DAT3

# Helper function to verify a file exists and has content
verify_file() {
	if [ ! -f "$1" ]; then
		echo "Error: $1 does not exist"
		exit 1
	fi
	if [ ! -s "$1" ]; then
		echo "Error: $1 is empty"
		exit 1
	fi
}

# ── Fixtures ──────────────────────────────────────────────────────────
# Downloaded once and kept in tests/ for reuse (CI caches them). All of
# these expect the caller's working directory to be tests/.

# Fallout 2 Restoration Project archive, ~130 MB DAT2 with ~13k files
RPU_VERSION="v2.4.33"
RPU_DAT="rpu.dat"
RPU_DAT_MD5="80fb4ba2bf94dfd60aeb89851400aefb"

fetch_rpu_dat() {
	local zip="rpu_${RPU_VERSION}.zip"
	if [ ! -f "$zip" ]; then
		wget -nv -O "$zip" \
			"https://github.com/BGforgeNet/Fallout2_Restoration_Project/releases/download/$RPU_VERSION/$zip"
	fi
	if [ ! -f "$RPU_DAT" ]; then
		unzip -j "$zip" "mods/$RPU_DAT"
	fi
	echo "$RPU_DAT_MD5  $RPU_DAT" | md5sum -c
}

# Extract the RPU archive and repack the result, leaving $2 next to $1's parent.
# Shared so both halves of the RPU test exercise the same archive.
build_rpu2_dat() {
	local src_dir="$1" out_dat="$2"
	rm -rf "$src_dir" "$out_dat"
	"$DAT3" x "$RPU_DAT" -o "$src_dir"
	(cd "$src_dir" && "$DAT3" a "../$out_dat" -- *)
}

# Arcanum demo's 30 MB archive (13k files). Only the extracted archive is
# kept; the demo installer is fetched and unpacked (RAR -> CAB -> DAT)
# solely to produce it.
ARCANUM_DEMO_DAT="ArcanumDemo.dat"
ARCANUM_DEMO_MD5="74e813d99a239c6ee04dd9fa807375a3"
ARCANUM_RAR_MD5="4a77a6cf6f801855cdfe0e7c61459a43"

fetch_arcanum_demo() {
	local rar="ArcanumDemo.rar"
	if [ ! -f "$ARCANUM_DEMO_DAT" ]; then
		if [ ! -f "$rar" ]; then
			wget -nv -O "$rar" "https://archive.org/download/ArcanumDemo/ArcanumDemo.rar"
		fi
		echo "$ARCANUM_RAR_MD5  $rar" | md5sum -c
		7z e -y "$rar" Setup1.cab
		7z e -y Setup1.cab Arcanum.dat
		mv Arcanum.dat "$ARCANUM_DEMO_DAT"
		rm -f Setup1.cab "$rar"
	fi
	echo "$ARCANUM_DEMO_MD5  $ARCANUM_DEMO_DAT" | md5sum -c
}

# TemplePlus' 3.7 MB ToEE archive (1,211 files plus 58 directory entries).
# The release ZIP is only a transport container: keep the DAT for the CI cache
# and discard the much larger ZIP after extracting it.
TEMPLEPLUS_VERSION="1.0.98"
TEMPLEPLUS_DAT="tpgamefiles.dat"
TEMPLEPLUS_DAT_SHA256="17bb15b4de3b0c551deb845ff6b8218c25b805f62a7791b153a3c34ab869a21a"
TEMPLEPLUS_ZIP_SHA256="45015817afeb29529f13ec8a62ff8f9d300e23ad1634ec7d2567b6a9b1ad5c97"

fetch_templeplus_dat() {
	local zip="TemplePlus-${TEMPLEPLUS_VERSION}.zip"
	if [ ! -f "$TEMPLEPLUS_DAT" ]; then
		if [ ! -f "$zip" ]; then
			wget -nv -O "$zip" \
				"https://github.com/GrognardsFromHell/TemplePlus/releases/download/v${TEMPLEPLUS_VERSION}/$zip"
		fi
		echo "$TEMPLEPLUS_ZIP_SHA256  $zip" | sha256sum -c
		# Info-ZIP returns 1 after successfully extracting a member whose ZIP
		# path uses backslashes, so accept that warning only if the DAT exists.
		unzip -j -o "$zip" 'tpdata\\tpgamefiles.dat' || [ -f "$TEMPLEPLUS_DAT" ]
		rm -f "$zip"
	fi
	echo "$TEMPLEPLUS_DAT_SHA256  $TEMPLEPLUS_DAT" | sha256sum -c
}

# Fallout 1 demo's 22 MB DAT1 archive (4.4k files). The retail Fallout 1
# archives cannot be redistributed, so this is the only real DAT1 the suite can
# fetch: 29 directories with files at the archive root as well as nested, and
# LZSS streams that end in a short raw block.
FALLOUT_DEMO_DAT="FalloutDemo.dat"
FALLOUT_DEMO_MD5="964ac33fe7c5dcd74123c4991f1ccadf"
FALLOUT_DEMO_ZIP_MD5="2f663c1509dafb6636011c766c39d786"

fetch_fallout_demo() {
	local zip="falldemo.zip"
	if [ ! -f "$FALLOUT_DEMO_DAT" ]; then
		if [ ! -f "$zip" ]; then
			wget -nv -O "$zip" "https://archive.org/download/FalloutDemo/falldemo.zip"
		fi
		echo "$FALLOUT_DEMO_ZIP_MD5  $zip" | md5sum -c
		unzip -j -o "$zip" "falldemo/Falldemo.dat"
		mv Falldemo.dat "$FALLOUT_DEMO_DAT"
		rm -f "$zip"
	fi
	echo "$FALLOUT_DEMO_MD5  $FALLOUT_DEMO_DAT" | md5sum -c
}

# Small deterministic source tree for the Arcanum tests: compressible files, a
# subdirectory, and a file so small it stores uncompressed (zlib overhead
# exceeds its size).
build_arcanum_src_tree() {
	local dir="$1"
	rm -rf "$dir"
	mkdir -p "$dir/data/sub"
	seq 1 2000 >"$dir/data/numbers.txt"
	printf 'hi\n' >"$dir/data/sub/tiny.txt"
	head -c 4096 /dev/zero >"$dir/data/sub/zeros.bin"
}

# ── Fallout 1 retail archives ─────────────────────────────────────────
# Game assets that cannot be redistributed, so the f1 tests run over whichever
# of these the user has dropped into tests/f1. They differ in what they can
# catch: critter.dat is a single directory of LZSS-compressed art with no raw
# blocks at all, while master.dat carries every content type the game ships,
# including the short trailing raw blocks that only appear in some streams.
F1_ARCHIVES=(critter.dat master.dat)

# Echoes the retail Fallout 1 archives present in tests/f1, one tests-relative
# path per line.
present_f1_archives() {
	local name
	for name in "${F1_ARCHIVES[@]}"; do
		if [ -f "$SCRIPT_DIR/f1/$name" ]; then
			echo "f1/$name"
		fi
	done
}

# Extract a DAT1 archive, repack the result as DAT1, and extract that: the two
# trees must be identical. $2 prefixes the scratch paths so several archives can
# round-trip in the same directory. Repacking stores entries uncompressed (dat3
# decompresses LZSS but does not produce it), so this exercises the reader
# against real game data and the writer against every shape that data has.
dat1_round_trip() {
	local archive="$1" prefix="$2"
	local src_dir="${prefix}_src" out_dat="${prefix}_repacked.dat" out_dir="${prefix}_out"

	rm -rf "$src_dir" "$out_dir" "$out_dat"
	"$DAT3" x "$archive" -o "$src_dir"
	(cd "$src_dir" && "$DAT3" a --format dat1 "../$out_dat" -- *)
	"$DAT3" x "$out_dat" -o "$out_dir"
	diff -qr "$src_dir" "$out_dir"
	rm -rf "$src_dir" "$out_dir" "$out_dat"
}

# Checksum every file under $1 into the file $2, in a stable order.
# -print0 throughout: some archive entries contain spaces ("rifle bb.frm").
hash_tree() {
	local dir="$1" out="$2"
	(cd "$dir" && LC_ALL=C find . -type f -print0 | LC_ALL=C sort -z | xargs -0 md5sum) >"$out"
}

# Tests needing an external runtime call one of these first: a missing runtime is
# a hard failure for them, while test.sh decides whether to run them at all.
require_runtime() {
	local runtime="$1" purpose="$2"
	if ! command -v "$runtime" >/dev/null 2>&1; then
		echo "Error: this test $purpose and needs $runtime" >&2
		exit 1
	fi
}

require_wine() {
	require_runtime wine "cross-checks against the original Windows binaries"
}

require_wasmtime() {
	require_runtime wasmtime "runs the WebAssembly build"
}

require_qemu_aarch64() {
	require_runtime qemu-aarch64-static "runs the arm64 build"
}
