#!/usr/bin/env bash
# Exercise scripts/release-version.sh against pure-Rust manifests.
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
temp_parent=$(cd "${TMPDIR:-/tmp}" && pwd -P)
fixture=$(mktemp -d "$temp_parent/mica-release-version.XXXXXX")
[[ "$fixture" == "$temp_parent"/mica-release-version.* ]] || exit 2
trap 'rm -rf -- "$fixture"' EXIT

tree=0.14.11
mkdir -p "$fixture/scripts" "$fixture/clients/mica_rust"
cp "$source_root/scripts/release-version.sh" "$source_root/scripts/manifest-version.sh" "$fixture/scripts/"
printf '[workspace.package]\nversion = "%s"\n' "$tree" > "$fixture/Cargo.toml"
printf '[package]\nname = "mica-rust"\nversion = "%s"\n' "$tree" > "$fixture/clients/mica_rust/Cargo.toml"

expect_refused() {
  local label=$1 needle=$2
  shift 2
  local out status=0
  out=$(resolve "$@" 2>&1) || status=$?
  [ "$status" != 0 ] || { echo "FAIL: $label was accepted" >&2; exit 1; }
  case "$out" in
    *"$needle"*) ;;
    *) echo "FAIL: $label did not mention '$needle': $out" >&2; exit 1 ;;
  esac
}

resolve() {
  (cd "$fixture" && bash scripts/release-version.sh "$@")
}

[ "$(resolve "$tree")" = "$tree" ] || { echo "FAIL: matching version refused" >&2; exit 1; }
expect_refused 'disagreeing version' '0.14.12' '0.14.12'
expect_refused 'leading v' 'v0.14.11' 'v0.14.11'
expect_refused 'two-part version' '0.14' '0.14'
expect_refused 'empty argument' 'X.Y.Z' ''

[ "$(GITHUB_REF_NAME=v$tree resolve)" = "$tree" ] || { echo 'FAIL: tag push refused' >&2; exit 1; }
GITHUB_REF_NAME=v0.14.12 expect_refused 'tag-push disagreement' '0.14.11'

echo 'PASS: pure-Rust shared version resolver passed all checks'
