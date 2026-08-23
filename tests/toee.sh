#!/bin/bash

set -xeu -o pipefail

# dat3 against a real ToEE archive shipped by TemplePlus: verify the reference
# tree, round-trip it, then modify a copy.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

TOEE_OUT="toee_dat3"
TOEE_MANIFEST="toee_dat3.md5"
TOEE_TREE_MD5="32b0dc2b41b3fb56151bd9b7ac25fa9a"
TOEE_REPACKED="toee_repacked.dat"
TOEE_REPACKED_OUT="toee_repacked_out"
TOEE_MOD="toee_mod.dat"
TOEE_MOD_OUT="toee_mod_out"
TOEE_DELETED="rules/indicators/sickened.txt"

fetch_templeplus_dat

# Listing and extraction exercise the real archive's raw and zlib-compressed
# entries. The checksum covers every extracted path and byte in stable order.
"$DAT3" l "$TEMPLEPLUS_DAT"
rm -rf "$TOEE_OUT"
"$DAT3" x "$TEMPLEPLUS_DAT" -o "$TOEE_OUT"
hash_tree "$TOEE_OUT" "$TOEE_MANIFEST"
echo "$TOEE_TREE_MD5  $TOEE_MANIFEST" | md5sum -c

# Repack the complete tree as ToEE and extract it again.
rm -f "$TOEE_REPACKED"
(cd "$TOEE_OUT" && "$DAT3" a --format toee -c 9 "../$TOEE_REPACKED" .)
rm -rf "$TOEE_REPACKED_OUT"
"$DAT3" x "$TOEE_REPACKED" -o "$TOEE_REPACKED_OUT"
diff -r "$TOEE_OUT" "$TOEE_REPACKED_OUT"

# Delete and add against a copy of the original archive.
cp "$TEMPLEPLUS_DAT" "$TOEE_MOD"
"$DAT3" d "$TOEE_MOD" "$TOEE_DELETED"
printf 'TemplePlus ToEE test file\n' >toee_add.txt
"$DAT3" a "$TOEE_MOD" -t rules toee_add.txt
rm -rf "$TOEE_MOD_OUT"
"$DAT3" x "$TOEE_MOD" -o "$TOEE_MOD_OUT"
diff "$TOEE_MOD_OUT/rules/toee_add.txt" toee_add.txt
if [ -e "$TOEE_MOD_OUT/$TOEE_DELETED" ]; then
	echo "Error: deleted file still present in archive"
	exit 1
fi

# The deleted name must now be rejected rather than silently ignored.
if "$DAT3" l "$TOEE_MOD" "$TOEE_DELETED" 2>/dev/null; then
	echo "Error: l should fail for a file that is not in the archive"
	exit 1
fi

# Keep the downloaded DAT for local and CI caches.
rm -rf "$TOEE_OUT" "$TOEE_MANIFEST" "$TOEE_REPACKED" \
	"$TOEE_REPACKED_OUT" "$TOEE_MOD" "$TOEE_MOD_OUT" toee_add.txt
