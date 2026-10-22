#!/bin/bash

set -xeu -o pipefail

# Smoke-test the macOS universal binary. Nothing on Linux can run it, so CI runs
# this on an Intel and an Apple Silicon runner, each exercising its own slice.
# MACOS_DAT3 is the binary to test.

# Without a native slice an Apple Silicon runner with Rosetta would run the
# x86_64 one instead and still pass
ARCH="$(uname -m)"
if ! lipo "${MACOS_DAT3:?set MACOS_DAT3 to the binary to test}" -verify_arch "$ARCH"; then
    echo "Error: $MACOS_DAT3 has no $ARCH slice" >&2
    exit 1
fi

"$(dirname "$0")/smoke.sh" "$MACOS_DAT3"
