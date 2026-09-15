#!/bin/bash

set -xeu -o pipefail

# Entry-name case: by default names match regardless of case and are listed,
# extracted and added in lowercase, except names that differ only in case,
# which keep their stored case and draw a warning. --case-sensitive works with
# names exactly as stored.

# shellcheck source=tests/common.sh
source "$(dirname "$0")/common.sh"

TEST_DIR="$SCRIPT_DIR/test_case_handling"

rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR"
# Removed on exit, so a failing check leaves nothing behind either
trap 'rm -rf "$TEST_DIR"' EXIT
cd "$TEST_DIR"

# Listings and warnings go to files: grep reading a pipe under pipefail would
# report dat3's or the writer's status, not whether the text was found.
expect_line() {
    local file="$1" name="$2"
    if ! grep -q " $name\$" "$file"; then
        echo "Error: expected $name in $file"
        cat "$file"
        exit 1
    fi
}

reject_line() {
    local file="$1" name="$2"
    if grep -q " $name\$" "$file"; then
        echo "Error: did not expect $name in $file"
        cat "$file"
        exit 1
    fi
}

mkdir -p src/Art
echo "hero" >src/Art/Hero.FRM
echo "upper" >src/README.TXT
echo "lower" >src/readme.txt

# Test 1: a default add stores lowercase names, even as seen case-sensitively
"$DAT3" a lower.dat -C src Art
"$DAT3" l --case-sensitive lower.dat >lower.list
expect_line lower.list art/hero.frm

# Test 2: --case-sensitive add keeps the source case
"$DAT3" a --case-sensitive mixed.dat -C src Art README.TXT
"$DAT3" l --case-sensitive mixed.dat >mixed_exact.list
expect_line mixed_exact.list Art/Hero.FRM
expect_line mixed_exact.list README.TXT

# Test 3: without the flag the same archive lists in lowercase, in text and JSON
"$DAT3" l mixed.dat >mixed.list 2>mixed.err
expect_line mixed.list art/hero.frm
reject_line mixed.list Art/Hero.FRM
"$DAT3" l --json mixed.dat >mixed.json
grep -q '"name": "art/hero.frm"' mixed.json
if [ -s mixed.err ]; then
    echo "Error: an archive without case-only twins should not warn"
    cat mixed.err
    exit 1
fi

# Test 4: default extraction writes lowercase paths; --case-sensitive keeps them
"$DAT3" x mixed.dat -o out_lower
verify_file out_lower/art/hero.frm
verify_file out_lower/readme.txt
if [ -e out_lower/Art ]; then
    echo "Error: default extraction should not create the stored-case directory"
    exit 1
fi
"$DAT3" e mixed.dat -o out_flat
verify_file out_flat/hero.frm
"$DAT3" x --case-sensitive mixed.dat -o out_exact
verify_file out_exact/Art/Hero.FRM
verify_file out_exact/README.TXT

# Test 5: names and globs match regardless of case by default only
"$DAT3" l mixed.dat ART/HERO.frm >match_plain.list
expect_line match_plain.list art/hero.frm
"$DAT3" l mixed.dat '*.frm' >match_glob.list
expect_line match_glob.list art/hero.frm
if "$DAT3" l --case-sensitive mixed.dat art/hero.frm; then
    echo "Error: --case-sensitive should not match a name in another case"
    exit 1
fi
if "$DAT3" l --case-sensitive mixed.dat '*.frm'; then
    echo "Error: --case-sensitive should not match a glob in another case"
    exit 1
fi

# Test 6: delete by a name in another case works by default only
cp mixed.dat delete.dat
if "$DAT3" d --case-sensitive delete.dat readme.txt; then
    echo "Error: --case-sensitive delete should need the exact case"
    exit 1
fi
"$DAT3" d delete.dat readme.txt
"$DAT3" l --case-sensitive delete.dat >delete.list
reject_line delete.list README.TXT

# Test 7: re-adding a name in another case replaces the entry by default
cp mixed.dat readd.dat
"$DAT3" a readd.dat -C src readme.txt
"$DAT3" l --case-sensitive readd.dat >readd.list
expect_line readd.list readme.txt
reject_line readd.list README.TXT

# ── Names differing only in case ──────────────────────────────────────

"$DAT3" a --case-sensitive twins.dat -C src README.TXT readme.txt Art

# Test 8: every command that opens the archive warns and names the twins
"$DAT3" l twins.dat >twins.list 2>twins_l.err
"$DAT3" x twins.dat -o out_twins 2>twins_x.err
"$DAT3" e twins.dat art/hero.frm -o out_twins_flat 2>twins_e.err
cp twins.dat twins_mod.dat
"$DAT3" a twins_mod.dat -C src/Art Hero.FRM 2>twins_a.err
"$DAT3" d twins_mod.dat art/hero.frm 2>twins_d.err
for err in twins_l.err twins_x.err twins_e.err twins_a.err twins_d.err; do
    if ! grep -q 'Warning: .*differ only in case' "$err"; then
        echo "Error: $err should carry the case-only duplicate warning"
        cat "$err"
        exit 1
    fi
    grep -q 'README.TXT / readme.txt' "$err"
done

# Test 9: the twins keep their stored case in listings and on disk
expect_line twins.list README.TXT
expect_line twins.list readme.txt
expect_line twins.list art/hero.frm
[ "$(cat out_twins/README.TXT)" = "upper" ]
[ "$(cat out_twins/readme.txt)" = "lower" ]
verify_file out_twins/art/hero.frm

# Test 10: a name matching either twin selects both
"$DAT3" l twins.dat readme.txt >twins_match.list 2>twins_match.err
expect_line twins_match.list README.TXT
expect_line twins_match.list readme.txt

# Test 11: --case-sensitive does not warn
"$DAT3" l --case-sensitive twins.dat >/dev/null 2>twins_exact.err
if [ -s twins_exact.err ]; then
    echo "Error: --case-sensitive should not warn about case-only twins"
    cat twins_exact.err
    exit 1
fi

echo "All case handling tests passed!"
