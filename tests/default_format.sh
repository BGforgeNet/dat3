#!/bin/bash

set -xeu -o pipefail

# .bgforge.yml's dat3.default_format picks the format of a new archive through
# the real binary: the config applies, --format overrides it, and a bad value
# warns and falls back to dat2.

# shellcheck source=tests/common.sh
source "$(dirname "$0")/common.sh"

TEST_DIR="$SCRIPT_DIR/test_default_format"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
# Removed on exit, so a failing check leaves nothing behind either
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$TEST_DIR"

echo "content" >file.txt

# Arcanum archives end in a footer whose magic sits 12 bytes from the end
is_arcanum() {
	[ "$(tail -c 12 "$1" | head -c 4)" = "1TAD" ]
}

# A DAT2 archive's last four bytes hold its own length, little-endian
is_dat2() {
	local size stored
	size=$(wc -c <"$1")
	stored=$(tail -c 4 "$1" | od -An -tu4 | tr -d ' ')
	[ "$size" -eq "$stored" ]
}

printf 'dat3:\n  default_format: arcanum\n' >.bgforge.yml

"$DAT3" a from_config.dat file.txt
if ! is_arcanum from_config.dat; then
	echo "Error: .bgforge.yml default_format arcanum did not produce an Arcanum archive"
	exit 1
fi

"$DAT3" a from_flag.dat --format dat2 file.txt
if is_arcanum from_flag.dat || ! is_dat2 from_flag.dat; then
	echo "Error: --format dat2 did not override .bgforge.yml"
	exit 1
fi

printf 'dat3:\n  default_format: zip\n' >.bgforge.yml
warning=$("$DAT3" a from_bad_config.dat file.txt 2>&1 >/dev/null)
if [[ "$warning" != *'unsupported dat3.default_format "zip"'* ]]; then
	echo "Error: an unsupported default_format was not reported: $warning"
	exit 1
fi
if ! is_dat2 from_bad_config.dat; then
	echo "Error: an unsupported default_format did not fall back to dat2"
	exit 1
fi

echo "Default format tests passed"
