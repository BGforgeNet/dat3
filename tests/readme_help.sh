#!/bin/bash

set -xeu -o pipefail

# README.md shows the top-level --help output. It is a copy of the clap
# definitions in crates/dat3/src/main.rs, so a changed flag or description
# would otherwise leave it silently stale.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

# The first block whose prompt line is a bare `dat3`, without the blank lines
# around the output. An empty result still differs from the help text below.
readme_help=$(awk 'copying && /^```$/ { exit } copying { print } /^dat3$/ { copying = 1 }' ../README.md | sed '/./,$!d')
help=$("$DAT3" --help)

if [ "$readme_help" != "$help" ]; then
    echo "Error: the --help block in README.md differs from dat3 --help:"
    # diff exits 1 on the difference it is here to show; the exit below reports it
    diff <(printf '%s\n' "$readme_help") <(printf '%s\n' "$help") || true
    exit 1
fi

echo "README help block test passed"
