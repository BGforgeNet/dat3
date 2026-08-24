#!/bin/bash

set -xeu -o pipefail

# Install the pinned gate tools when missing or out of date
./install-tools.sh actionlint cargo-audit cargo-deny cargo-machete zizmor

# Workflow YAML lint. zizmor gates at "low" and above; the one finding below
# that is a style note preferring `gh release` over the pinned release action,
# which the project keeps for its fail_on_unmatched_files check.
actionlint
zizmor --min-severity low .github/workflows/

# Typecheck the TypeScript test helpers under tests/
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
