#!/bin/bash

set -xeu -o pipefail

# Build static release for tests
cargo build --release --target x86_64-unknown-linux-musl

# Integration tests
cd tests
./non-ascii.sh
./rpu.sh

# Put Fallout 1 critter.dat into tests/f1 to run this
if [ -f f1/critter.dat ]; then
    ./f1.sh
fi

# Response file test
./response_file.sh

# Add validation test
./add_validation.sh

# Duplicate paths test
./duplicate_paths.sh

# Path consistency test
./path_consistency.sh

# Glob pattern handling test
./glob_handling.sh
