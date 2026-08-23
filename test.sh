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
# Runs against the demo archive it fetches; put retail Fallout 1 archives into
# tests/f1 to widen it. The script says which of those it found.
./f1.sh
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
# cross-compiled dat3.exe from build.sh. f1_wine.sh needs retail Fallout 1
# archives in tests/f1 and says so when it finds none.
run_if_available wine ./rpu_wine.sh ./arcanum_wine.sh ./glob_handling_wine.sh ./f1_wine.sh
