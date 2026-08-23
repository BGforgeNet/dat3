#!/bin/bash

set -xeu -o pipefail

# Install the pinned gate tools when missing or out of date
./install-tools.sh cargo-audit cargo-deny cargo-machete

# Typecheck the TypeScript integration test (tests/json_listing.ts)
npm ci
npm run typecheck

# Format check
cargo fmt --all -- --check

# Clippy lints, test targets included - without --all-targets the #[cfg(test)]
# modules are never compiled under clippy
cargo clippy --all-targets -- -D warnings

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
