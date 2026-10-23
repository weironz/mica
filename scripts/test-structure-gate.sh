#!/usr/bin/env bash
# Exercise the real gate and CI base selector in disposable Git repositories.
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
temp_parent=$(cd "${TMPDIR:-/tmp}" && pwd)
fixture_root=$(mktemp -d "$temp_parent/mica-structure-gate.XXXXXX")
# Only remove the directory this invocation just created under the resolved temp
# directory, never a repository/worktree or a path supplied by a caller.
[[ "$fixture_root" == "$temp_parent"/mica-structure-gate.* ]] || exit 2
trap 'rm -rf -- "$fixture_root"' EXIT
passed=0

new_repo() {
  repo=$(mktemp -d "$fixture_root/repo.XXXXXX")
  mkdir -p "$repo/scripts" "$repo/crates/demo/src" "$repo/clients/mica_flutter/lib" "$repo/clients/mica_flutter/rust/src"
  cp "$source_root/scripts/check-file-size.sh" "$source_root/scripts/check-structure-ci.sh" "$repo/scripts/"
  git -C "$repo" init -q
  git -C "$repo" config user.name 'Structure gate fixture'
  git -C "$repo" config user.email 'structure-gate@example.invalid'
  git -C "$repo" config core.autocrlf false
}

lines() {
  mkdir -p "$(dirname "$repo/$1")"
  awk -v count="$2" 'BEGIN { for (i=0; i<count; i++) print "fixture line" }' > "$repo/$1"
}

# A Dart library root: $1 = path, $2 = prelude lines (the `part '…';`
# declarations), $3 = how many fixture lines follow them.
lines_decl() {
  mkdir -p "$(dirname "$repo/$1")"
  printf '%s\n' "$2" > "$repo/$1"
  awk -v count="$3" 'BEGIN { for (i=0; i<count; i++) print "fixture line" }' >> "$repo/$1"
}

commit() {
  git -C "$repo" add .
  git -C "$repo" commit -qm fixture
}

expect() {
  local expected=$1 name=$2 status=0
  shift 2
  (cd "$repo" && "$@") > "$fixture_root/output" 2>&1 || status=$?
  if [[ "$status" != "$expected" ]]; then
    echo "FAIL fixture: $name (expected $expected, got $status)" >&2
    cat "$fixture_root/output" >&2
    exit 1
  fi
  passed=$((passed + 1))
  echo "PASS fixture: $name"
}

new_repo
lines crates/demo/src/nested/lib.rs 2001
commit
base=$(git -C "$repo" rev-parse HEAD)
lines crates/demo/src/nested/lib.rs 2301
expect 0 'Rust +300 boundary passes' bash scripts/check-file-size.sh --diff "$base"
lines crates/demo/src/nested/lib.rs 2302
expect 1 'Rust +301 is rejected' bash scripts/check-file-size.sh --diff "$base"

new_repo
lines clients/mica_flutter/lib/nested/view.dart 2001
commit
base=$(git -C "$repo" rev-parse HEAD)
lines clients/mica_flutter/lib/nested/view.dart 2302
expect 1 'nested Dart +301 is rejected' bash scripts/check-file-size.sh --diff "$base"

new_repo
for path in docs/design.md crates/demo/tests/fixtures/golden.json crates/demo/tests/integration.rs notes.txt; do
  lines "$path" 2001
done
commit
base=$(git -C "$repo" rev-parse HEAD)
for path in docs/design.md crates/demo/tests/fixtures/golden.json crates/demo/tests/integration.rs notes.txt; do
  lines "$path" 2302
done
expect 0 'documents, JSON fixtures, tests outside src and text are outside the gate' bash scripts/check-file-size.sh --diff "$base"

new_repo
lines clients/mica_flutter/lib/src/rust/generated.dart 2001
commit
base=$(git -C "$repo" rev-parse HEAD)
lines clients/mica_flutter/lib/src/rust/generated.dart 8100
lines crates/demo/src/new.rs 2302
expect 0 'generated exemptions and new split files remain allowed' bash scripts/check-file-size.sh --diff "$base"
lines crates/demo/src/new.rs 8001
expect 1 'absolute limit still rejects oversized new source' bash scripts/check-file-size.sh --diff "$base"

# A Dart library is root + parts, summed: splitting into parts cannot bypass the
# absolute limit (#87). Root 3 lines (1 declaration + 2 fixture) + part 7,999
# = 8,002 — every single file is under the limit, the unit is not.
new_repo
lines_decl clients/mica_flutter/lib/monolith.dart "part 'monolith_extra.dart';" 2
lines clients/mica_flutter/lib/monolith_extra.dart 7999
commit
expect 1 'parts of one library are summed against the absolute limit' bash scripts/check-file-size.sh

# A registered library above the hard limit passes under the ratchet — and the
# ratchet catches growth landing in ANY of its parts.
new_repo
lines_decl clients/mica_flutter/lib/monolith.dart "part 'monolith_extra.dart';" 2
lines clients/mica_flutter/lib/monolith_extra.dart 9000
commit
echo '9003 clients/mica_flutter/lib/monolith.dart # fixture library ratchet' > "$repo/scripts/file-size-budget.txt"
expect 0 'a registered library above the hard limit passes under the ratchet' bash scripts/check-file-size.sh
lines clients/mica_flutter/lib/monolith_extra.dart 9001
expect 1 'the library ratchet rejects growth landing in any part' bash scripts/check-file-size.sh

# The FFI crate is inside the gate (#87): hand-written sources are checked,
# frb_generated.rs (flutter_rust_bridge output) stays exempt.
new_repo
lines clients/mica_flutter/rust/src/frb_generated.rs 9000
commit
expect 0 'generated frb bindings stay exempt' bash scripts/check-file-size.sh
lines clients/mica_flutter/rust/src/api/big.rs 8001
expect 1 'hand-written FFI crate sources are inside the gate' bash scripts/check-file-size.sh

# The delta gate aggregates per unit: root +200 and part +200 of the SAME
# library (each under the +300 file limit of the old gate) sum to +400.
new_repo
lines_decl clients/mica_flutter/lib/app.dart "part 'app_more.dart';" 2
lines clients/mica_flutter/lib/app_more.dart 2100
commit
base=$(git -C "$repo" rev-parse HEAD)
lines_decl clients/mica_flutter/lib/app.dart "part 'app_more.dart';" 202
lines clients/mica_flutter/lib/app_more.dart 2300
expect 1 'the delta gate aggregates root and part growth in one library' bash scripts/check-file-size.sh --diff "$base"

new_repo
lines crates/demo/src/lib.rs 2001
commit
before=$(git -C "$repo" rev-parse HEAD)
lines crates/demo/src/lib.rs 2302
commit
lines crates/demo/src/lib.rs 2303
commit
# Simulate checkout after main has advanced: origin/main and HEAD are identical.
git -C "$repo" update-ref refs/remotes/origin/main HEAD
expect 0 'latest-commit-only baseline would miss the same growth' bash scripts/check-file-size.sh --diff HEAD^
expect 1 'PR base retains feature-branch growth checks' env GITHUB_EVENT_NAME=pull_request STRUCTURE_PR_BASE="$before" bash scripts/check-structure-ci.sh

new_repo
lines crates/demo/src/lib.rs 2001
commit
lines crates/demo/src/lib.rs 2302
commit
expect 1 'unavailable PR base falls back to previous commit' env GITHUB_EVENT_NAME=pull_request STRUCTURE_PR_BASE=deadbeef bash scripts/check-structure-ci.sh
expect 1 'manual dispatch falls back to previous commit' env GITHUB_EVENT_NAME=workflow_dispatch bash scripts/check-structure-ci.sh

new_repo
lines crates/demo/src/lib.rs 8001
commit
expect 1 'initial history still enforces absolute limits' env GITHUB_EVENT_NAME=workflow_dispatch bash scripts/check-structure-ci.sh
lines crates/demo/src/lib.rs 10
commit
echo '9 crates/demo/src/lib.rs # fixture ratchet' > "$repo/scripts/file-size-budget.txt"
expect 1 'ratchet still rejects growth against its budget' bash scripts/check-file-size.sh

echo "structure gate fixtures: $passed passed"
