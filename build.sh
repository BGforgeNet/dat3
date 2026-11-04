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
    wasm32-wasip1
)

# mimalloc is C, and no aarch64-musl C compiler ships in apt; zig provides one.
# macOS needs a Mach-O linker, which zig also provides, and cargo-zigbuild's
# universal2 target merges the x86_64 and arm64 builds into one binary.
ZIG_TARGETS=(
    aarch64-unknown-linux-musl
    universal2-apple-darwin
)

# Windows 7 and later. Rust's own Windows targets require Windows 10; the win7
# ones are tier 3, with no prebuilt standard library, so a pinned nightly builds
# it from source. MinGW links them (gcc-mingw-w64-x86-64-win32 and
# gcc-mingw-w64-i686-win32 on Debian and Ubuntu): zig links only the Universal C
# Runtime, which Windows 7 has only after an optional update, and MinGW links
# msvcrt.dll, which every Windows ships.
WIN7_TOOLCHAIN=nightly-2026-10-28
WIN7_TARGETS=(
    x86_64-win7-windows-gnu
    i686-win7-windows-gnu
)

ALL_TARGETS=("${CARGO_TARGETS[@]}" "${ZIG_TARGETS[@]}" "${WIN7_TARGETS[@]}")

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
win7_toolchain_needed=""
for target in "${targets[@]}"; do
    case "$target" in
    # Not a rustup target: the two it merges are
    universal2-apple-darwin) rustup_targets+=(x86_64-apple-darwin aarch64-apple-darwin) ;;
    # Tier 3: no rustup target, the standard library comes from rust-src
    *-win7-windows-gnu) win7_toolchain_needed=1 ;;
    *) rustup_targets+=("$target") ;;
    esac
done
if [ -n "$win7_toolchain_needed" ]; then
    rustup toolchain install "$WIN7_TOOLCHAIN" --profile minimal --component rust-src
fi
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
    elif [[ " ${WIN7_TARGETS[*]} " == *" $target "* ]]; then
        # panic_abort for the release profile's panic = "abort"
        start_build "$profile $target" cargo "+$WIN7_TOOLCHAIN" build "${profile_args[@]}" \
            -Z build-std=std,panic_abort --target "$target"
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
