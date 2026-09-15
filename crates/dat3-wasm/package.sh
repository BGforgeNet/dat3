#!/bin/bash

set -xeu -o pipefail

# Builds the dat3-wasm npm package: the crate compiled for wasm32-unknown-unknown,
# wasm-bindgen's nodejs glue, and package.json stamped with the workspace version.
# Leaves the package in target/dat3-wasm/pkg and its tarball at target/dat3-wasm.tgz.

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PKG_DIR="$ROOT/target/dat3-wasm/pkg"
TARBALL="$ROOT/target/dat3-wasm.tgz"

cd "$ROOT"

# The glue only works with the crate version it was generated for, so a stale
# or missing CLI fails here rather than producing a package that breaks at load.
wanted="$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' crates/dat3-wasm/Cargo.toml)"
if [[ "$(wasm-bindgen --version)" != "wasm-bindgen $wanted" ]]; then
    echo "Error: wasm-bindgen $wanted is required (./install-tools.sh wasm-bindgen)" >&2
    exit 1
fi

cargo build --release --target wasm32-unknown-unknown -p dat3-wasm

rm -rf "$PKG_DIR"
# --weak-refs frees wasm memory held by an Archive once JS garbage-collects it
wasm-bindgen --target nodejs --weak-refs --out-dir "$PKG_DIR" \
    target/wasm32-unknown-unknown/release/dat3_wasm.wasm

version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)"
if [ -z "$version" ]; then
    echo "Error: no version found in the workspace Cargo.toml" >&2
    exit 1
fi
cp crates/dat3-wasm/package.json "$PKG_DIR/package.json"
(cd "$PKG_DIR" && npm pkg set "version=$version")

# npm pack names the tarball after name and version; the release asset keeps one fixed name
pack_dir="$(mktemp -d)"
npm pack "$PKG_DIR" --pack-destination "$pack_dir"
mv "$pack_dir/dat3-wasm-$version.tgz" "$TARBALL"
rmdir "$pack_dir"
