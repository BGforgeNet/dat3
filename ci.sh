#!/bin/bash

set -xeuo pipefail

cargo install cargo-binstall

cargo binstall -y cargo-audit cargo-deny cargo-machete cargo-bloat

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
