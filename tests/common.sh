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

# Checksum every file under $1 into the file $2, in a stable order.
# -print0 throughout: some archive entries contain spaces ("rifle bb.frm").
hash_tree() {
	local dir="$1" out="$2"
	(cd "$dir" && LC_ALL=C find . -type f -print0 | LC_ALL=C sort -z | xargs -0 md5sum) >"$out"
}

# Wine cross-check scripts call this first: a missing wine is a hard failure
# for them, while test.sh decides whether to run them at all.
require_wine() {
	if ! command -v wine >/dev/null 2>&1; then
		echo "Error: this test cross-checks against Windows binaries and needs wine" >&2
		exit 1
	fi
}
