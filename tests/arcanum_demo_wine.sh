#!/bin/bash

set -xeu -o pipefail

# Real game data: dat3 and Troika's own dbmaker.exe both extract the Arcanum
# demo archive, and the trees must match.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

require_wine

fetch_arcanum_demo

# dbmaker keeps the demo's stored mixed-case names (WorldMap), so dat3 does too
# for every step compared against it.
rm -rf demo_dat3 demo_db
$DAT3 x --case-sensitive "$ARCANUM_DEMO_DAT" -o demo_dat3
mkdir demo_db
(cd demo_db && dbmaker -u "../$ARCANUM_DEMO_DAT")
diff -r demo_dat3 demo_db

# Clean up (keep the extracted DAT for the CI cache)
rm -rf demo_dat3 demo_db
