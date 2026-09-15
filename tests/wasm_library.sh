#!/bin/bash

set -xeu -o pipefail

# Builds the dat3-wasm npm package, then runs its integration test, which
# installs the tarball the way an application would. With DAT3_PREBUILT set the
# tarball already under target/ is tested instead, as CI does with the one its
# build job ships.

cd "$(dirname "$0")"

# shellcheck source=tests/common.sh
source ./common.sh

# The test reads these without fetching them
fetch_rpu_dat
fetch_arcanum_demo
fetch_templeplus_dat
fetch_fallout_demo

if [ -z "${DAT3_PREBUILT:-}" ]; then
    ../crates/dat3-wasm/package.sh
fi
node ./wasm_library.ts
