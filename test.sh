#!/bin/bash

set -xeu -o pipefail

# Build static release for tests
cargo build --release --target x86_64-unknown-linux-musl

# The WebAssembly module is pure Rust, so building it here needs no toolchain
# beyond the rustup target. Skipped when nothing can run it.
if command -v wasmtime >/dev/null 2>&1; then
	cargo build --release --target wasm32-wasip1
fi

# Runs the given scripts, or explains why it did not. A local box may lack the
# runtime, but CI must never quietly skip anything.
run_if_available() {
	local runtime="$1"
	local script
	shift
	if command -v "$runtime" >/dev/null 2>&1; then
		for script in "$@"; do
			"$script"
		done
	elif [ -n "${CI:-}" ]; then
		echo "Error: $runtime is not installed; these checks cannot be skipped in CI: $*" >&2
		exit 1
	else
		echo "SKIPPED ($runtime not installed): $*"
	fi
}

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
# TypeScript: the assertions are about a parsed document, so a real parser runs them
node ./json_listing.ts

# The WebAssembly build, run under a WASI runtime, and the arm64 build under
# qemu. The arm64 binary comes from build.sh, which needs zig to produce it.
run_if_available wasmtime ./wasm.sh
run_if_available qemu-aarch64-static ./arm64.sh

# Cross-checks against the original Windows tools. These need the
# cross-compiled dat3.exe from build.sh.
WINE_TESTS=(./rpu_wine.sh ./arcanum_wine.sh ./glob_handling_wine.sh)
if [ -f f1/critter.dat ]; then
	WINE_TESTS+=(./f1_wine.sh)
fi

run_if_available wine "${WINE_TESTS[@]}"
