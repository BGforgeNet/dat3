#!/bin/bash

set -xeu -o pipefail

# Every command that writes to stdout must survive the reader closing the pipe
# early. `l` has handled this since v0.6.0; `x` and `a` aborted with SIGABRT and
# a core dump, which no other archiver does - tar, zip, unzip and 7z all exit
# 141 (death by SIGPIPE) on the same input.

# Work inside tests directory
cd "$(dirname "$0")"

# Load common variables and functions
# shellcheck source=tests/common.sh
source ./common.sh

SRC_DIR="pipe_src"
PIPE_DAT="pipe_test.dat"
OUT_DIR="pipe_out"

# Enough entries that the reader has certainly exited before the writer's
# second line: progress is reported every 1000 files.
rm -rf "$SRC_DIR" "$OUT_DIR" "$PIPE_DAT"
mkdir -p "$SRC_DIR/data"
for i in $(seq 1 5000); do
	printf 'x' >"$SRC_DIR/data/f$i.txt"
done

# Asserts that $1 (a description) run as the remaining arguments does not die
# to a signal other than SIGPIPE when its stdout closes after one line.
assert_survives_closed_pipe() {
	local what="$1"
	shift
	local code=0
	"$@" | head -1 || code=$?
	# 0 (guard exits cleanly) and 141 (SIGPIPE) are both fine; 128+n for any
	# other n means the process died to a signal - 134 is the SIGABRT panic.
	if [ "$code" -ge 128 ] && [ "$code" -ne 141 ]; then
		echo "Error: $what died on a closed pipe with exit $code"
		exit 1
	fi
}

# Surviving is only half of it: a closed progress channel must not abandon the
# work, so each command is also checked to have finished what it was asked to do.

assert_survives_closed_pipe "a (add)" "$DAT3" a "$PIPE_DAT" -C "$SRC_DIR" data
verify_file "$PIPE_DAT"
if [ "$("$DAT3" l --json "$PIPE_DAT" | grep -c '"name"')" -ne 5000 ]; then
	echo "Error: a (add) did not store every file when its stdout closed early"
	exit 1
fi

assert_survives_closed_pipe "x (extract)" "$DAT3" x "$PIPE_DAT" -o "$OUT_DIR"
if [ "$(find "$OUT_DIR" -type f | wc -l)" -ne 5000 ]; then
	echo "Error: x (extract) did not write every file when its stdout closed early"
	exit 1
fi

assert_survives_closed_pipe "d (delete)" "$DAT3" d "$PIPE_DAT" "data/f1.txt"
# Output discarded deliberately: this call is expected to FAIL, and its "not
# found" message on stderr is the success condition, not a problem to report.
if "$DAT3" l "$PIPE_DAT" "data/f1.txt" >/dev/null 2>&1; then
	echo "Error: d (delete) did not persist when its stdout closed early"
	exit 1
fi

# l already handled this; keep it covered so the guard cannot regress there.
assert_survives_closed_pipe "l (list)" "$DAT3" l "$PIPE_DAT"

rm -rf "$SRC_DIR" "$OUT_DIR" "$PIPE_DAT"
