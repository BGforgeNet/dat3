#!/bin/bash

set -xeu -o pipefail

# Builds the dat3-wasm npm package, then runs its integration test, which
# installs the tarball the way an application would.

cd "$(dirname "$0")"
../crates/dat3-wasm/package.sh
node ./wasm_library.ts
