#!/usr/bin/env bash
# Run the headless test suite and report a trustworthy exit code.
#
# Why this exists: `cargo test ... | grep -c FAILED` looks like a summary but
# lies in two independent ways.
#
#   1. `grep -c` prints 0 when nothing matches but still exits 1, so a pipeline
#      chained with `&&` reports failure on a fully green suite.
#   2. Piping through `tee` (or any command) makes the pipeline's exit status
#      that of the LAST command, so a genuinely failing `cargo test` is reported
#      as success whenever the trailing filter succeeds.
#
# Both directions have actually happened in this repository. This script reads
# the exit status from PIPESTATUS[0] and prints the counts separately, so a real
# failure can never be swallowed and a green run can never be called red.
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$REPO/plnk/Cargo.toml"
LOG="${TMPDIR:-/tmp}/plnk-test-$$.log"

cargo test --manifest-path "$MANIFEST" 2>&1 | tee "$LOG"
cargo_rc=${PIPESTATUS[0]}

passed=$(awk -F'[.;]' '/^test result:/ {gsub(/ /,"",$2); s+=$2} END {print s+0}' "$LOG")
targets=$(grep -c 'test result: ok' "$LOG" || true)
failed=$(grep -c 'FAILED' "$LOG" || true)
ignored=$(awk -F'[.;]' '/^test result:/ {gsub(/ /,"",$4); s+=$4} END {print s+0}' "$LOG")

echo "---"
echo "cargo exit : $cargo_rc"
echo "targets ok : $targets"
echo "passed     : $passed"
echo "ignored    : $ignored"
echo "FAILED     : $failed"

if [ "$cargo_rc" -ne 0 ]; then
    echo "RESULT: FAIL (cargo exited $cargo_rc)"
    rm -f "$LOG"
    exit 1
fi
if [ "$failed" -ne 0 ]; then
    echo "RESULT: FAIL (the log contains FAILED)"
    echo "log kept: $LOG"
    exit 1
fi
echo "RESULT: PASS"
rm -f "$LOG"
exit 0
