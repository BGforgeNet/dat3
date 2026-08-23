#!/bin/bash

set -xeu -o pipefail

echo "Cross-compiling static binaries for all platforms..."

# Targets whose toolchain is either bundled with rustc or already on PATH.
CARGO_TARGETS=(
	x86_64-unknown-linux-musl
	x86_64-pc-windows-gnu
	i686-pc-windows-gnu
	wasm32-wasip1
)

# mimalloc is C, and no aarch64-musl C compiler ships in apt; zig provides one.
# Only this target needs it, so the others stay on plain cargo.
ZIG_TARGETS=(
	aarch64-unknown-linux-musl
)

ALL_TARGETS=("${CARGO_TARGETS[@]}" "${ZIG_TARGETS[@]}")

# Install targets if not already installed. Tolerated failure: a distro rustc has
# no rustup, and its targets come from packages instead. A missing target still
# fails loudly at the cargo build below.
for target in "${ALL_TARGETS[@]}"; do
	rustup target add "$target" 2>/dev/null || true
done

# Build all targets in parallel - both debug and release
echo "Building debug and release targets..."
for target in "${CARGO_TARGETS[@]}"; do
	cargo build --target "$target" &
	cargo build --release --target "$target" &
done
for target in "${ZIG_TARGETS[@]}"; do
	cargo zigbuild --target "$target" &
	cargo zigbuild --release --target "$target" &
done

# Wait for all builds to complete
wait

# Binary name for a target: Windows appends .exe, wasm produces a module
binary_name() {
	case "$1" in
	*-windows-*) echo "dat3.exe" ;;
	wasm32-*) echo "dat3.wasm" ;;
	*) echo "dat3" ;;
	esac
}

echo ""
echo "Cross-compile completed. Static binaries:"
for profile in debug release; do
	echo "$profile builds:"
	for target in "${ALL_TARGETS[@]}"; do
		ls -lh "target/$target/$profile/$(binary_name "$target")"
	done
	echo ""
done
