#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
selector="$repo/scripts/ci-paths-for-scope.sh"
all=$'rust\nflutter\nweb-e2e\ncontainer\nmcp'

check() {
  local label=$1 expected=$2
  shift 2
  local got
  got=$(printf '%s\0' "$@" | bash "$selector" --stdin)
  [ "$got" = "$expected" ] || {
    printf 'FAIL %s\nexpected:\n%s\ngot:\n%s\n' "$label" "$expected" "$got" >&2
    exit 1
  }
}

[ "$(bash "$selector")" = "$all" ]
check docs '' docs/ops/release.md README.md
check flutter $'flutter\nweb-e2e' clients/mica_flutter/e2e/web_harness_entry.dart
check lockfile $'flutter\nweb-e2e' clients/mica_flutter/pubspec.lock
check sdk $'flutter\nweb-e2e' .fvmrc
check backend $'rust\nweb-e2e\ncontainer\nmcp' crates/api-server/src/main.rs
check mixed "$all" crates/mica-core/src/lib.rs clients/mica_flutter/lib/main.dart
check ffi "$all" clients/mica_flutter/rust/src/lib.rs
check workflow "$all" .github/workflows/ci.yml
check unknown "$all" new-module/build.config
check nginx $'web-e2e\ncontainer' deploy/nginx.conf
check spaced $'flutter\nweb-e2e' 'clients/mica_flutter/test/fixture with spaces.dart'

tmp_root=$(cd "${TMPDIR:-/tmp}" && pwd -P)
fixture=$(mktemp -d "$tmp_root/mica-ci-paths.XXXXXX")
fixture=$(cd "$fixture" && pwd -P)
case "$fixture" in "$tmp_root"/mica-ci-paths.*) ;; *) exit 1 ;; esac
trap 'rm -rf -- "$fixture"' EXIT
(
  cd "$fixture"
  git init -q
  git config user.email fixture@mica.test
  git config user.name fixture
  mkdir -p crates/api-server/src docs
  printf 'fixture\n' > crates/api-server/src/example.rs
  git add .
  git commit -qm initial
  base=$(git rev-parse HEAD)
  git mv crates/api-server/src/example.rs docs/example.md
  git commit -qm move
  [ "$(bash "$selector" "$base" HEAD)" = $'rust\nweb-e2e\ncontainer\nmcp' ]
  if bash "$selector" no-such-ref HEAD >/dev/null 2>&1; then
    echo 'FAIL: invalid baseline must fail instead of selecting no jobs' >&2
    exit 1
  fi
)
echo 'CI job selection: full, mixed changes, pins, renames and invalid baselines passed'
