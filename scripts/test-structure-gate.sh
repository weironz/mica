#!/usr/bin/env bash
# Regression tests for scripts/check-file-size.sh (pure-Rust single-file gate).
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

new_repo() {
  local repo=$1
  mkdir -p "$repo/scripts" "$repo/crates/demo/src" "$repo/clients/mica_rust/src"
  cp "$source_root/scripts/check-file-size.sh" "$repo/scripts/"
  : > "$repo/scripts/file-size-budget.txt"
  (
    cd "$repo"
    git init -q
    git config user.email test@mica.dev
    git config user.name test
  )
}

lines() {
  local file=$1 count=$2
  mkdir -p "$(dirname "$file")"
  awk -v n="$count" 'BEGIN{for(i=0;i<n;i++) print "// line"}' > "$file"
}

expect() {
  local want=$1 label=$2
  shift 2
  set +e
  "$@" >/dev/null 2>&1
  local got=$?
  set -e
  if [ "$got" -ne "$want" ]; then
    echo "FAIL: $label (expected exit $want, got $got)" >&2
    exit 1
  fi
}

repo="$tmp/r1"
new_repo "$repo"
lines "$repo/crates/demo/src/ok.rs" 3000
lines "$repo/clients/mica_rust/src/view.rs" 2001
(
  cd "$repo"
  git add . && git commit -qm base
  base=$(git rev-parse HEAD)
  expect 0 'under hard limit passes' bash scripts/check-file-size.sh
  lines "$repo/clients/mica_rust/src/view.rs" 2302
  git add . && git commit -qm grow
  expect 1 'client Rust file >2000 net +301 rejected' bash scripts/check-file-size.sh --diff "$base"
  lines "$repo/clients/mica_rust/src/huge.rs" 8001
  expect 1 'Rust file >8000 hard limit rejected' bash scripts/check-file-size.sh
  echo '8005 clients/mica_rust/src/huge.rs' > "$repo/scripts/file-size-budget.txt"
  expect 0 'ratchet baseline allows registered file' bash scripts/check-file-size.sh
  lines "$repo/clients/mica_rust/src/huge.rs" 8006
  expect 1 'growing past ratchet baseline rejected' bash scripts/check-file-size.sh
)
echo 'PASS: pure-Rust file-size structure gate regression suite'
