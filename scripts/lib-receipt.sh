# Test receipts: a gate passes only if the tests it names actually RAN.
# Sourced after scripts/lib-local.sh (uses fail / note).

# cargo_receipt MIN cargo test ARGS...
# Runs the command, then sums every `test result: ok. N passed` line.
cargo_receipt() {
  local min=$1 log rc passed
  shift
  log=$(mktemp)
  set +e
  "$@" 2>&1 | tee "$log"
  rc=${PIPESTATUS[0]}
  set -e
  passed=$(grep -oE '^test result: [A-Za-z]+\. [0-9]+ passed' "$log" | awk '{s += $4} END {print s + 0}')
  rm -f "$log"
  [ "$rc" -eq 0 ] || fail "'$*' exited $rc"
  [ "$passed" -ge "$min" ] \
    || fail "'$*' passed $passed test(s), expected at least $min — a filter that matches nothing is not a pass"
  note "receipt: $passed passed (floor $min)"
}
