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
  mkdir -p "$repo/scripts" "$repo/crates/demo/src" "$repo/clients/mica_flutter/lib"
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
expect 1 'push.before covers all commits in a main push' env GITHUB_EVENT_NAME=push STRUCTURE_PUSH_BEFORE="$before" bash scripts/check-structure-ci.sh
expect 0 'latest-commit-only baseline would miss the same growth' bash scripts/check-file-size.sh --diff HEAD^
expect 1 'PR base retains feature-branch growth checks' env GITHUB_EVENT_NAME=pull_request STRUCTURE_PR_BASE="$before" bash scripts/check-structure-ci.sh

new_repo
lines crates/demo/src/lib.rs 2001
commit
lines crates/demo/src/lib.rs 2302
commit
expect 1 'missing push SHA falls back to previous commit' env GITHUB_EVENT_NAME=push STRUCTURE_PUSH_BEFORE=deadbeef bash scripts/check-structure-ci.sh
expect 1 'zero push SHA falls back to previous commit' env GITHUB_EVENT_NAME=push STRUCTURE_PUSH_BEFORE=0000000000000000000000000000000000000000 bash scripts/check-structure-ci.sh
expect 1 'unavailable PR base falls back to previous commit' env GITHUB_EVENT_NAME=pull_request STRUCTURE_PR_BASE=deadbeef bash scripts/check-structure-ci.sh

new_repo
lines crates/demo/src/lib.rs 8001
commit
expect 1 'initial history still enforces absolute limits' env GITHUB_EVENT_NAME=push STRUCTURE_PUSH_BEFORE=0000000000000000000000000000000000000000 bash scripts/check-structure-ci.sh
lines crates/demo/src/lib.rs 10
commit
echo '9 crates/demo/src/lib.rs # fixture ratchet' > "$repo/scripts/file-size-budget.txt"
expect 1 'ratchet still rejects growth against its budget' bash scripts/check-file-size.sh

echo "structure gate fixtures: $passed passed"
