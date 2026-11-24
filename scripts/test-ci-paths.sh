#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
selector="$repo/scripts/ci-paths-for-scope.sh"
all=$'rust\nclient\ncontainer\nmcp'

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
check client $'client\ncontainer' clients/mica_rust/src/editor_view.rs
check backend $'rust\ncontainer\nmcp' crates/api-server/src/main.rs
check shared-core "$all" crates/mica-editor/src/lib.rs
check md-core "$all" crates/md-core/src/lib.rs
check workflow "$all" .github/workflows/ci.yml
check unknown "$all" new-module/build.config
check deploy 'container' deploy/nginx.conf
echo 'CI job selection (pure Rust): passed'
