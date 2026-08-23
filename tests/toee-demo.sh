#!/bin/bash

set -xeu -o pipefail

# CI-only coverage for every ToEE DAT in the official 218 MB demo. The smaller
# TemplePlus fixture remains in test.sh so local runs do not fetch this installer.

cd "$(dirname "$0")"

# shellcheck source=tests/common.sh
source ./common.sh

TOEE_DEMO_DIR="toee-demo"
TOEE_DEMO_URL="https://archive.org/download/ToEEDemo/ToEE_Demo.exe"
TOEE_DEMO_INSTALLER_SHA256="827a280487c3615ae4be0185daefbb63098a93c71f6a21409446040969e8c6a9"

# label|fixture path|archive sha256|files|compressed files|unpacked bytes|delete probe
TOEE_DEMO_ARCHIVES=(
	"ToEE1|ToEE1.dat|15d960a32a0a3c4096a5bf2de256c3019cf64f4e96313d89ebdd698b02ddcf33|8812|8113|218966341|art/BadArt.jpg"
	"ToEE2|ToEE2.dat|c176e77ed1f7a6af4fc0cec467ce1777281e5f488dc5bbffc26540ee5339bce3|1343|184|43491603|art/splash/legal0322.img"
	"ToEE3|ToEE3.dat|7a0b2168008291753dc02b847562aab7ddca1a6d0bafb874c22a58f9836c02be|2678|1540|30060497|dlg/00001test.dlg"
	"tig|tig.dat|29a160866ea70b50563d57bc113cacd9ca490b9859a37704e18c1d762f6aed67|23|22|265579|art/arial-10/arial-10.bmp"
	"module-ToEE|modules/ToEE.dat|ecfc24b37d3869bc2b506ae76ab7ce579fccc564b82e4fc439cef0fe2f5f5f2b|3031|2971|28001479|art/ground/ground.mes"
)

scratch="$(mktemp -d /tmp/dat3-toee-demo.XXXXXX)"
trap 'rm -rf "$scratch"' EXIT

# Entry-table count, which unlike `l` also covers directory entries: the footer's
# last word is the table's distance from EOF, and the table opens with the count.
toee_entry_count() {
	local file=$1 size distance table
	size=$(stat -c%s "$file")
	distance=$(od -An -tu4 -j $((size - 4)) -N4 "$file" | tr -d ' ')
	table=$((size - distance))
	od -An -tu4 -j "$table" -N4 "$file" | tr -d ' '
}

demo_files_present() {
	local spec label relative sha files compressed unpacked deleted
	for spec in "${TOEE_DEMO_ARCHIVES[@]}"; do
		IFS='|' read -r label relative sha files compressed unpacked deleted <<<"$spec"
		if [ ! -f "$TOEE_DEMO_DIR/$relative" ]; then
			return 1
		fi
	done
}

fetch_demo_files() {
	local installer unpacked

	require_runtime wget "downloads the ToEE demo installer"
	require_runtime node "splits the installer into its cabinet volumes"
	require_runtime unshield "extracts the InstallShield cabinets"

	installer="$scratch/ToEE_Demo.exe"
	unpacked="$scratch/unpacked"

	wget -nv -O "$installer" "$TOEE_DEMO_URL"
	echo "$TOEE_DEMO_INSTALLER_SHA256  $installer" | sha256sum -c

	# The installer is an uncompressed container, so splitting it needs no
	# decoder: ./iss_extract.ts copies out data1.hdr and the data*.cab volumes,
	# which unshield then reads as one set from that directory.
	node ./iss_extract.ts "$installer" "$scratch/installer"
	mkdir -p "$unpacked"
	unshield -d "$unpacked" x "$scratch/installer/data1.cab" '*.dat'

	rm -rf "$TOEE_DEMO_DIR"
	mkdir -p "$TOEE_DEMO_DIR/modules"
	cp "$unpacked/English/ToEE1.dat" "$TOEE_DEMO_DIR/ToEE1.dat"
	cp "$unpacked/English/ToEE2.dat" "$TOEE_DEMO_DIR/ToEE2.dat"
	cp "$unpacked/English/ToEE3.dat" "$TOEE_DEMO_DIR/ToEE3.dat"
	cp "$unpacked/English/tig.dat" "$TOEE_DEMO_DIR/tig.dat"
	cp "$unpacked/English/modules/ToEE.dat" "$TOEE_DEMO_DIR/modules/ToEE.dat"
}

if ! demo_files_present; then
	fetch_demo_files
fi

require_runtime node "checks the ToEE demo JSON listings"

for spec in "${TOEE_DEMO_ARCHIVES[@]}"; do
	IFS='|' read -r label relative sha files compressed unpacked deleted <<<"$spec"
	archive="$TOEE_DEMO_DIR/$relative"
	list="$scratch/$label.json"
	original_out="$scratch/$label-original"
	repacked="$scratch/$label-repacked.dat"
	repacked_out="$scratch/$label-repacked"
	modified="$scratch/$label-modified.dat"
	modified_out="$scratch/$label-modified"
	probe="$scratch/probe.txt"

	echo "$sha  $archive" | sha256sum -c
	"$DAT3" l "$archive" --json >"$list"
	actual="$(node ./listing_summary.ts "$list")"
	if [ "$actual" != "$files|$compressed|$unpacked" ]; then
		echo "Error: unexpected listing summary for $relative: $actual" >&2
		exit 1
	fi

	# Read the original, then create a new ToEE archive from scratch and make
	# auto-detection reopen it before comparing every extracted path and byte.
	"$DAT3" x "$archive" -o "$original_out" >/dev/null
	(cd "$original_out" && "$DAT3" a --format toee -c 9 "$repacked" . >/dev/null)
	"$DAT3" x "$repacked" -o "$repacked_out" >/dev/null
	diff -r "$original_out" "$repacked_out"

	# Rewrite a copy of the real archive and ensure detection retains ToEE v1,
	# including its identity GUID, rather than treating the shared magic as Arcanum.
	before_guid="$(tail -c 28 "$archive" | head -c 16 | sha256sum)"
	before_entries="$(toee_entry_count "$archive")"
	cp "$archive" "$modified"
	"$DAT3" d "$modified" "$deleted" >/dev/null
	printf 'dat3 ToEE demo probe\n' >"$probe"
	"$DAT3" a "$modified" -t dat3-demo-probe "$probe" >/dev/null
	if "$DAT3" l "$modified" "$deleted" >/dev/null 2>&1; then
		echo "Error: deleted ToEE demo entry is still present: $deleted" >&2
		exit 1
	fi
	"$DAT3" x "$modified" dat3-demo-probe/probe.txt -o "$modified_out" >/dev/null
	cmp "$probe" "$modified_out/dat3-demo-probe/probe.txt"
	after_guid="$(tail -c 28 "$modified" | head -c 16 | sha256sum)"
	if [ "$before_guid" != "$after_guid" ] || [ "$(tail -c 12 "$modified" | head -c 4)" != "1TAD" ]; then
		echo "Error: ToEE v1 footer identity changed for $relative" >&2
		exit 1
	fi

	# One entry removed, one file plus its new directory added. Every other
	# directory must survive, including the ones holding no files at all, which
	# a rewrite that re-derives the tree from file paths alone would drop.
	after_entries="$(toee_entry_count "$modified")"
	if [ "$after_entries" -ne "$((before_entries + 1))" ]; then
		echo "Error: $relative entry count went $before_entries -> $after_entries," \
			"expected $((before_entries + 1)); directory entries were lost" >&2
		exit 1
	fi
done
