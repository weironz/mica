#!/usr/bin/env bash
# The audit wrapper must inspect both graphs and propagate either failure.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/bin"
cat > "$fixture/bin/cargo-audit" <<'STUB'
#!/usr/bin/env bash
exit 0
STUB
cat > "$fixture/bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$AUDIT_TEST_CALLS"
[[ " $* " == *' --deny warnings '* ]] || exit 90
[[ -z "${AUDIT_FAIL_LOCK:-}" || " $* " != *" --file $AUDIT_FAIL_LOCK "* ]] || exit 91
STUB
chmod +x "$fixture/bin/"*
export AUDIT_TEST_CALLS="$fixture/calls"
PATH="$fixture/bin:$PATH" bash scripts/check-rust-audit.sh > "$fixture/output"
[ "$(wc -l < "$AUDIT_TEST_CALLS")" -eq 2 ]
grep -Fq -- '--file Cargo.lock --deny warnings' "$AUDIT_TEST_CALLS"
grep -Fq -- '--file clients/mica_flutter/rust/Cargo.lock --deny warnings' "$AUDIT_TEST_CALLS"
for lock in Cargo.lock clients/mica_flutter/rust/Cargo.lock; do
  if PATH="$fixture/bin:$PATH" AUDIT_FAIL_LOCK="$lock" bash scripts/check-rust-audit.sh > "$fixture/output" 2>&1; then
    echo "FAIL: ignored audit failure in $lock" >&2
    exit 1
  fi
done
echo 'Rust audit wrapper: both lockfiles scanned, warnings denied, failures propagated'
