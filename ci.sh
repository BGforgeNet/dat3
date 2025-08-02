#!/bin/bash

set -xeu -o pipefail

cargo install cargo-binstall

# Caching action has a bug, binstalls are not cached properly.
if ! cargo help audit >/dev/null 2>&1; then
    cargo binstall -y --force cargo-audit cargo-deny cargo-machete cargo-bloat
fi

# Format check
cargo fmt --all -- --check

# Clippy lints
cargo clippy -- -D warnings

# Compilation check
cargo check

# Tests
cargo test --verbose

# Security audit
cargo audit

# License/dependency check
cargo deny check licenses
cargo deny check advisories
cargo deny check bans

# Unused dependencies
cargo machete
