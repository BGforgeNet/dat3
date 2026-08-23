#!/bin/bash

set -xeu -o pipefail

# Build static release for tests
cargo build --release --target x86_64-unknown-linux-musl

cd tests

# dat3-only tests
./non-ascii.sh
./rpu.sh
./arcanum.sh

# Put Fallout 1 critter.dat into tests/f1 to run this
if [ -f f1/critter.dat ]; then
	./f1.sh
fi

./response_file.sh
./add_validation.sh
./duplicate_paths.sh
./path_consistency.sh
./glob_handling.sh
./extract_missing.sh

# Cross-checks against the original Windows tools. These need wine and the
# cross-compiled dat3.exe from build.sh. A local box may not have wine, but CI
# must never quietly skip them.
WINE_TESTS=(./rpu_wine.sh ./arcanum_wine.sh ./glob_handling_wine.sh)
if [ -f f1/critter.dat ]; then
	WINE_TESTS+=(./f1_wine.sh)
fi

if command -v wine >/dev/null 2>&1; then
	for wine_test in "${WINE_TESTS[@]}"; do
		"$wine_test"
	done
elif [ -n "${CI:-}" ]; then
	echo "Error: wine is not installed; the Windows cross-checks cannot be skipped in CI" >&2
	exit 1
else
	echo "SKIPPED (wine not installed): ${WINE_TESTS[*]}"
fi
