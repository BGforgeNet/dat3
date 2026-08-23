#!/bin/bash

set -xeu -o pipefail

# dat3 against the DAT1 format: round-trip the Fallout 1 demo's archive, plus
# any retail archive the user has dropped into f1/. The dat2.exe cross-checks
# live in f1_wine.sh.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

# -- Real game data: the Fallout 1 demo's 22 MB archive (4.4k files) --

fetch_fallout_demo

$DAT3 l "$FALLOUT_DEMO_DAT" >/dev/null
dat1_round_trip "$FALLOUT_DEMO_DAT" f1_demo

# -- Retail archives, when the user has supplied them --

archives="$(present_f1_archives)"
if [ -z "$archives" ]; then
	echo "SKIPPED (no retail archives in f1/): ${F1_ARCHIVES[*]}"
	exit 0
fi

for archive in $archives; do
	dat1_round_trip "$archive" "f1_$(basename "$archive" .dat)"
done
