#!/bin/bash

set -xeu -o pipefail

# Common variables and functions for test scripts

# Use static build - get absolute path relative to this script's location
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DAT3="$SCRIPT_DIR/../target/x86_64-unknown-linux-musl/release/dat3"
export DAT3

# Helper function to verify a file exists and has content
verify_file() {
	if [ ! -f "$1" ]; then
		echo "Error: $1 does not exist"
		exit 1
	fi
	if [ ! -s "$1" ]; then
		echo "Error: $1 is empty"
		exit 1
	fi
}
