#!/usr/bin/env bash
# Exercise scripts/release-version.sh, the ONE resolver every job in
# release.yml shares, in a disposable Git fixture with a known tree version.
#
# The regression (#119): each producing job resolved the version for itself from
# `inputs.version`, and `images` had no gate at all — so when `draft` refused
# ("asked for 0.13.69, this tree says 0.13.68"), `images` still built and pushed
# 0.13.69 tags under publish=true. Moving resolution AND the tree check into one
# script, which only `draft` runs and the other jobs consume as `needs: draft`
# outputs, is what the YAML wiring test could only assert by reading source; this
# tests the behaviour itself.
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
temp_parent=$(cd "${TMPDIR:-/tmp}" && pwd -P)
fixture=$(mktemp -d "$temp_parent/mica-release-version.XXXXXX")
[[ "$fixture" == "$temp_parent"/mica-release-version.* ]] || exit 2
trap 'rm -rf -- "$fixture"' EXIT

tree=0.13.68
mkdir -p "$fixture/scripts" "$fixture/clients/mica_flutter/lib"
cp "$source_root/scripts/release-version.sh" "$source_root/scripts/manifest-version.sh" "$fixture/scripts/"
printf '[package]\nname = "mica-cli"\nversion = "%s"\n' "$tree" > "$fixture/Cargo.toml"
printf 'version: %s\n' "$tree" > "$fixture/clients/mica_flutter/pubspec.yaml"
printf "const kAppVersion = '%s';\n" "$tree" > "$fixture/clients/mica_flutter/lib/main.dart"
expect_refused() { # $1 = label, $2 = needle, rest = resolver args
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

resolve() { # $@ = args; env GITHUB_REF_NAME already set by caller
  (cd "$fixture" && bash scripts/release-version.sh "$@")
}


# ── the workflow_dispatch path: an explicit version ─────────────────────────
[ "$(resolve "$tree")" = "$tree" ] \
  || { echo "FAIL: an explicit version matching the tree must be accepted" >&2; exit 1; }
expect_refused 'a version that disagrees with the tree' '0.13.69' '0.13.69'
expect_refused 'a leading v' 'v0.13.68' 'v0.13.68'
expect_refused 'a two-part version' '0.13' '0.13'
expect_refused 'a four-part version' '0.13.68.1' '0.13.68.1'
expect_refused 'a non-numeric version' 'abc' 'abc'
expect_refused 'an empty argument' 'X.Y.Z' ''

# ── the tag-push path: the version comes off the tag name ───────────────────
[ "$(GITHUB_REF_NAME=v$tree resolve)" = "$tree" ] \
  || { echo 'FAIL: a tag push must read the version off GITHUB_REF_NAME' >&2; exit 1; }
expect_refused 'a tag that disagrees with the tree' '0.13.69' '0.13.69'
GITHUB_REF_NAME=v0.13.69 expect_refused 'the tag-push disagreement' '0.13.68'
expect_refused 'no version and no ref name' 'X.Y.Z'

echo 'PASS: the shared version resolver accepts only what the tree agrees with, on both trigger paths'
