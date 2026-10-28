# Test receipts: a gate passes only if the tests it names actually RAN.
# Sourced after scripts/lib-local.sh (uses fail / note / $FLUTTER).
#
# An exit code of 0 proves nothing ran badly; it does not prove anything ran.
# Seen here: `cargo test <filter>` that matches nothing exits 0; a browser-only
# suite run on the VM compiles to its stub and "passes" with zero tests; a
# DB-backed test with no database returns early and reports passed. Each of
# these looks exactly like a green run. So these wrappers count what passed
# and refuse a count below the floor the caller states.

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

# flutter_receipt MIN ARGS...   (ARGS go to `$FLUTTER test`)
# Counts tests that finished successfully, excluding skipped ones and the
# hidden "loading <file>" pseudo-tests every suite reports.
flutter_receipt() {
  local min=$1 report report_arg rc passed
  shift
  report=$(mktemp)
  # Flutter here is a NATIVE Windows process: it reads `/tmp/x` as `C:\tmp\x`,
  # while mktemp wrote to MSYS /tmp (%TEMP%). The report then lands somewhere
  # else and this function sees an EMPTY file — "0 passed" refuses a run whose
  # tests all passed (seen 2026-10-28). Hand the reporter a path the child
  # process resolves to the same file. No-op on Linux/macOS.
  if command -v cygpath >/dev/null 2>&1; then
    report_arg=$(cygpath -w "$report")
  else
    report_arg=$report
  fi
  set +e
  "$FLUTTER" test --file-reporter "json:$report_arg" "$@"
  rc=$?
  set -e
  passed=$(grep '"type":"testDone"' "$report" | grep '"hidden":false' | grep '"skipped":false' \
    | grep -c '"result":"success"' || true)
  rm -f "$report"
  [ "$rc" -eq 0 ] || fail "'flutter test $*' exited $rc"
  [ "$passed" -ge "$min" ] \
    || fail "'flutter test $*' passed $passed test(s), expected at least $min — skipped or stubbed out is not a pass"
  note "receipt: $passed passed (floor $min)"
}
