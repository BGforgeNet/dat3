#!/bin/bash

set -xeu -o pipefail

# The declared rust-version is a promise to users, and the other gates build on a
# toolchain many releases newer: code using a feature stabilised after it would
# pass them and fail for anyone honouring docs/building.md. The version is read
# from Cargo.toml rather than repeated here.
msrv="$(sed -n 's/^rust-version = "\(.*\)"/\1/p' Cargo.toml)"
if [ -z "$msrv" ]; then
    echo "Error: no rust-version in Cargo.toml" >&2
    exit 1
fi

rustup toolchain install "$msrv" --profile minimal --target wasm32-unknown-unknown

# --locked so the committed lockfile is what gets checked, and --all-targets so
# the test code is covered too
cargo "+$msrv" check --all-targets --locked
# Not a default workspace member: it builds only for its wasm target
cargo "+$msrv" check -p dat3-wasm --target wasm32-unknown-unknown --locked
