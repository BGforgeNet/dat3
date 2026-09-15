#!/bin/bash

set -xeu -o pipefail

# Real game data: dat3 modifies the Arcanum demo archive, and Troika's own
# dbmaker.exe reads the result.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

require_wine

fetch_arcanum_demo

# The demo stores mixed-case names (WorldMap), which dbmaker keeps
DEMO_MOD="demo_mod.dat"
cp "$ARCANUM_DEMO_DAT" "$DEMO_MOD"
$DAT3 d --case-sensitive "$DEMO_MOD" "WorldMap/WorldMap.mes"
echo "demo test file" >demo_add.txt
$DAT3 a --case-sensitive "$DEMO_MOD" -t WorldMap demo_add.txt
rm -rf demo_mod_db
mkdir demo_mod_db
(cd demo_mod_db && dbmaker -u "../$DEMO_MOD")
diff demo_mod_db/WorldMap/demo_add.txt demo_add.txt
if [ -e demo_mod_db/WorldMap/WorldMap.mes ]; then
	echo "Error: deleted file still present in archive"
	exit 1
fi

# Clean up (keep the extracted DAT for the CI cache)
rm -rf demo_mod_db "$DEMO_MOD" demo_add.txt
