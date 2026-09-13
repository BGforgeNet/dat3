#!/bin/bash

set -xeu -o pipefail

# Install the pinned gate tools when missing or out of date
./install-tools.sh actionlint cargo-deny cargo-machete shellcheck shfmt zizmor

# Workflow YAML lint. zizmor gates at "low" and above; the one finding below
# that is a style note preferring `gh release` over the pinned release action,
# which the project keeps for its fail_on_unmatched_files check.
actionlint
zizmor --min-severity low .github/workflows/

# Shell scripts: lint at shellcheck's default severity, and fail on any formatting drift
git ls-files -z '*.sh' | xargs -0 shellcheck
git ls-files -z '*.sh' | xargs -0 shfmt -d

# Typecheck the TypeScript test helpers under tests/
npm ci
npm run typecheck

# Format check
cargo fmt --all -- --check

# Clippy lints, test targets included - without --all-targets the #[cfg(test)]
# modules are never compiled under clippy
cargo clippy --all-targets -- -D warnings
# dat3-wasm is not a default member: it builds only for its wasm target
cargo clippy -p dat3-wasm --target wasm32-unknown-unknown -- -D warnings

# Tests
cargo test --verbose

# Library API docs: a broken doc link or other rustdoc warning fails the build
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --package dat3-core

# License, advisory (RustSec) and duplicate-dependency checks
cargo deny check -D parse-error licenses
cargo deny check advisories
cargo deny check bans

# Unused dependencies
cargo machete
