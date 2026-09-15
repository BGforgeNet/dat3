#!/bin/bash

set -xeu -o pipefail

# Usage: ./test.sh [test...]   (default: every test below, in order)
#        ./test.sh --list      (prints every test's name)
# CI splits the suite across parallel jobs by naming tests. There DAT3_PREBUILT=1
# tests the binaries a build job produced instead of building them, and each test
# that passed is appended to the file DAT3_TESTS_RAN names, which a final job
# checks against --list so a test no job names fails the run.

# test|runtime it needs beyond the native build ("-" for none, "ci" for a test
# that runs only in CI). The runtime ones explain their skip locally, but CI must
# never quietly skip anything.
TESTS=(
    "non-ascii.sh|-"
    "rpu.sh|-"
    "arcanum.sh|-"
    "toee.sh|-"
    # Runs against the demo archive it fetches; put retail Fallout 1 archives into
    # tests/f1 to widen it. The script says which of those it found.
    "f1.sh|-"
    "broken_pipe.sh|-"
    "response_file.sh|-"
    "add_validation.sh|-"
    "duplicate_paths.sh|-"
    "path_consistency.sh|-"
    "glob_handling.sh|-"
    "extract_missing.sh|-"
    "default_format.sh|-"
    "case_handling.sh|-"
    "readme_help.sh|-"
    # TypeScript: the assertions are about a parsed document, so a real parser runs
    # them. Node is required for the suite, so a missing one fails rather than skips.
    "json_listing.ts|-"
    # Every ToEE archive of the official demo, whose 218 MB installer local runs
    # do not fetch; toee.sh covers the format locally
    "toee-demo.sh|ci"
    # The npm package for Node and Electron, installed as an application would and
    # checked against the native build on the fixtures. Building the package is
    # what needs wasm-bindgen; a prebuilt one needs only node.
    "wasm_library.sh|wasm-bindgen"
    # The WebAssembly build, run under a WASI runtime, and the arm64 build under
    # qemu. The arm64 binary comes from build.sh, which needs zig to produce it.
    "wasm.sh|wasmtime"
    "arm64.sh|qemu-aarch64-static"
    # Cross-checks against the original Windows tools. glob_handling_wine.sh needs
    # the cross-compiled dat3.exe from build.sh. f1_wine.sh needs retail Fallout 1
    # archives in tests/f1 and says so when it finds none.
    "rpu_wine.sh|wine"
    "arcanum_wine.sh|wine"
    "arcanum_demo_wine.sh|wine"
    "arcanum_demo_mod_wine.sh|wine"
    "glob_handling_wine.sh|wine"
    "f1_wine.sh|wine"
)

# Prints the runtime a test needs, or nothing for an unknown name
runtime_of() {
    local spec
    for spec in "${TESTS[@]}"; do
        if [ "${spec%%|*}" = "$1" ]; then
            echo "${spec#*|}"
            return 0
        fi
    done
}

if [ "${1:-}" = --list ]; then
    for spec in "${TESTS[@]}"; do
        echo "${spec%%|*}"
    done
    exit 0
fi

selected=("$@")
if [ ${#selected[@]} -eq 0 ]; then
    for spec in "${TESTS[@]}"; do
        selected+=("${spec%%|*}")
    done
fi
# Every name is checked before anything runs, so a typo fails at once rather
# than after the tests ahead of it
for name in "${selected[@]}"; do
    if [ -z "$(runtime_of "$name")" ]; then
        echo "Error: no such test: $name (./test.sh --list names them)" >&2
        exit 1
    fi
done

if [ -z "${DAT3_PREBUILT:-}" ]; then
    # Build static release for tests
    cargo build --release --target x86_64-unknown-linux-musl

    # The WebAssembly module is pure Rust, so building it here needs no toolchain
    # beyond the rustup target. Skipped when nothing can run it.
    if command -v wasmtime >/dev/null 2>&1; then
        cargo build --release --target wasm32-wasip1
    fi
fi

# Resolved before the cd below, which would otherwise re-root a relative path
ran_file=""
if [ -n "${DAT3_TESTS_RAN:-}" ]; then
    ran_file="$(realpath -m "$DAT3_TESTS_RAN")"
fi

cd tests

for name in "${selected[@]}"; do
    runtime="$(runtime_of "$name")"
    if [ -n "${DAT3_PREBUILT:-}" ] && [ "$runtime" = wasm-bindgen ]; then
        runtime=node
    fi
    if [ "$runtime" = ci ]; then
        if [ -z "${CI:-}" ]; then
            echo "SKIPPED (runs only in CI): $name"
            continue
        fi
    elif [ "$runtime" != - ] && ! command -v "$runtime" >/dev/null 2>&1; then
        if [ -n "${CI:-}" ]; then
            echo "Error: $runtime is not installed; this check cannot be skipped in CI: $name" >&2
            exit 1
        fi
        echo "SKIPPED ($runtime not installed): $name"
        continue
    fi
    case "$name" in
    *.ts) node "./$name" ;;
    *) "./$name" ;;
    esac
    if [ -n "$ran_file" ]; then
        echo "$name" >>"$ran_file"
    fi
done
