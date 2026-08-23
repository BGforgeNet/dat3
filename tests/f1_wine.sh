#!/bin/bash

set -xeu -o pipefail

# Cross-check the DAT1 format against the original dat2.exe, over any retail
# Fallout 1 archive the user has dropped into f1/. The dat3-only checks live in
# f1.sh.
#
# The demo archive f1.sh fetches is deliberately not cross-checked here: it
# stores files at the archive root as well as in directories, and dat2.exe's
# handling of root-level DAT1 entries has not been established, so wiring it in
# would gate the suite on unverified reference behaviour.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

require_wine

export WINEDEBUG=-all
# wine writes startup noise to stderr even with WINEDEBUG=-all, so the dat2.exe
# calls below discard it. Their exit status still gates under set -e, and every
# extraction is checked by the diff that follows it.
DAT2="wine ./dat2.exe"

archives="$(present_f1_archives)"
if [ -z "$archives" ]; then
	echo "SKIPPED (no retail archives in f1/): ${F1_ARCHIVES[*]}"
	exit 0
fi

for archive in $archives; do
	prefix="f1_$(basename "$archive" .dat)"
	ref_dir="${prefix}_ref"
	dat3_dir="${prefix}_dat3"
	repacked="${prefix}_repacked.dat"
	repacked_dir="${prefix}_repacked_out"

	# dat2.exe's extraction is the reference both halves compare against; it is
	# the slow step, so it is kept between runs.
	if [ ! -d "$ref_dir" ]; then
		$DAT2 x -d "$ref_dir" "$archive" 2>/dev/null
	fi

	# Test 1: dat3 must extract what dat2.exe extracts
	rm -rf "$dat3_dir"
	$DAT3 x "$archive" -o "$dat3_dir"
	diff -qr "$ref_dir" "$dat3_dir"

	# Test 2: dat2.exe must read back an archive dat3 wrote
	rm -rf "$repacked" "$repacked_dir"
	(cd "$dat3_dir" && $DAT3 a --format dat1 "../$repacked" -- *)
	$DAT2 x -d "$repacked_dir" "$repacked" 2>/dev/null
	diff -qr "$ref_dir" "$repacked_dir"

	rm -rf "$dat3_dir" "$repacked" "$repacked_dir"
done
