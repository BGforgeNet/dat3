#!/bin/bash

set -xeu -o pipefail

# Common variables and functions for test scripts

# Use static build - get absolute path from tests directory
SCRIPT_DIR="$(dirname "$(realpath "$0")")"
DAT3="$(realpath "$SCRIPT_DIR/../target/x86_64-unknown-linux-musl/release/dat3")"
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
