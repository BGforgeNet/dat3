#!/bin/bash

set -xeu -o pipefail

# Install cargo-binstall if not present (pinned for Rust 1.88 compatibility)
if ! command -v cargo-binstall &>/dev/null; then
	cargo install cargo-binstall@1.10.0
fi

# Install CI tools with pinned versions if not present
if ! command -v cargo-audit &>/dev/null; then
	cargo binstall -y cargo-audit@0.22.0 cargo-deny@0.19.0 cargo-machete@0.9.1 cargo-bloat@0.12.1
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
cargo deny check -D parse-error licenses
cargo deny check advisories
cargo deny check bans

# Unused dependencies
cargo machete
