#!/bin/bash

set -xeu -o pipefail

# Usage: ./build.sh [--debug] [target...]   (default: every target below and the npm package)
# Release builds, or debug builds with --debug. "npm" names the npm package, whose
# own target is wasm32-unknown-unknown, and is always a release build. CI splits
# the targets across jobs, so they build in parallel on separate runners.

echo "Cross-compiling static binaries..."

# Targets whose toolchain is either bundled with rustc or already on PATH.
CARGO_TARGETS=(
    x86_64-unknown-linux-musl
    x86_64-pc-windows-gnu
    i686-pc-windows-gnu
    wasm32-wasip1
)

# mimalloc is C, and no aarch64-musl C compiler ships in apt; zig provides one.
# macOS needs a Mach-O linker, which zig also provides, and cargo-zigbuild's
# universal2 target merges the x86_64 and arm64 builds into one binary.
ZIG_TARGETS=(
    aarch64-unknown-linux-musl
    universal2-apple-darwin
)

ALL_TARGETS=("${CARGO_TARGETS[@]}" "${ZIG_TARGETS[@]}")

profile=release
if [ "${1:-}" = --debug ]; then
    profile=debug
    shift
fi

targets=()
build_npm=""
if [ $# -eq 0 ]; then
    targets=("${ALL_TARGETS[@]}")
    build_npm=1
fi
for arg in "$@"; do
    if [ "$arg" = npm ]; then
        build_npm=1
    elif [[ " ${ALL_TARGETS[*]} " == *" $arg "* ]]; then
        targets+=("$arg")
    else
        echo "Error: no such target: $arg (known: ${ALL_TARGETS[*]} npm)" >&2
        exit 1
    fi
done

# Install targets if not already installed. Tolerated failure: a distro rustc has
# no rustup, and its targets come from packages instead. A missing target still
# fails loudly at the cargo build below.
rustup_targets=()
for target in "${targets[@]}"; do
    case "$target" in
    # Not a rustup target: the two it merges are
    universal2-apple-darwin) rustup_targets+=(x86_64-apple-darwin aarch64-apple-darwin) ;;
    *) rustup_targets+=("$target") ;;
    esac
done
# wasm32-unknown-unknown is the npm package's target, built by its own script below.
if [ -n "$build_npm" ]; then
    rustup_targets+=(wasm32-unknown-unknown)
fi
for target in "${rustup_targets[@]}"; do
    rustup target add "$target" 2>/dev/null || true
done

# Build all targets in parallel.
# pids and labels stay global on purpose: start_build appends to them.
pids=()
labels=()

start_build() {
    local label="$1"
    shift
    "$@" &
    pids+=("$!")
    labels+=("$label")
}

profile_args=()
if [ "$profile" = release ]; then
    profile_args=(--release)
fi

echo "Building $profile targets..."
for target in "${targets[@]}"; do
    if [[ " ${ZIG_TARGETS[*]} " == *" $target "* ]]; then
        start_build "$profile $target" cargo zigbuild "${profile_args[@]}" --target "$target"
    else
        start_build "$profile $target" cargo build "${profile_args[@]}" --target "$target"
    fi
done

# Waited per pid, not with a bare `wait`: that reports 0 whatever the jobs did,
# leaving a failed build to be caught only by the ls below - which a stale
# binary from a restored target/ cache would satisfy, shipping it as a release
# asset.
for i in "${!pids[@]}"; do
    if ! wait "${pids[$i]}"; then
        echo "Error: ${labels[$i]} build failed" >&2
        exit 1
    fi
done

# Binary name for a target: Windows appends .exe, wasm produces a module
binary_name() {
    case "$1" in
    *-windows-*) echo "dat3.exe" ;;
    wasm32-*) echo "dat3.wasm" ;;
    *) echo "dat3" ;;
    esac
}

# The npm package for Node and Electron
if [ -n "$build_npm" ]; then
    crates/dat3-wasm/package.sh
fi

# Lists an output and, when DAT3_BUILD_OUTPUTS names a file, appends its path
# there; CI packs exactly those paths into the job's artifact.
output() {
    ls -lh "$1"
    if [ -n "${DAT3_BUILD_OUTPUTS:-}" ]; then
        echo "$1" >>"$DAT3_BUILD_OUTPUTS"
    fi
}

echo ""
echo "Cross-compile completed. Static $profile binaries:"
for target in "${targets[@]}"; do
    output "target/$target/$profile/$(binary_name "$target")"
done
echo ""
if [ -n "$build_npm" ]; then
    echo "npm package:"
    output target/dat3-wasm.tgz
fi
