#!/usr/bin/env bash
# Flip the draft release vX.Y.Z public and mark it latest — only when every
# REQUIRED asset is attached.
#
#   just release-publish 0.13.57
#   REQUIRE_MACOS=1 just release-publish 0.13.57   # also insist on the macOS CLI
#
# The local twin of the `publish` job in release.yml. The installer is what the
# in-app updater downloads from /releases/latest, so publishing without it
# would offer an update that cannot install; the check below is that guarantee.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=${1:-}
check_version_arg "$version"
tag="v$version"
require_gh

assets=$(gh release view "$tag" --repo "$GH_REPO" --json assets --jq '.assets[].name') \
  || fail "no release $tag — run 'just release-upload $version' first"

required=(
  "Mica-Setup-$version.exe"
  "mica-cli-$version-windows-x64.exe"
  "mica-cli-$version-linux-x64"
)
[ "${REQUIRE_MACOS:-}" = 1 ] && required+=("mica-cli-$version-macos-arm64")

missing=()
for a in "${required[@]}"; do
  grep -qxF "$a" <<<"$assets" || missing+=("$a")
done
[ ${#missing[@]} -eq 0 ] || fail "not attached yet: ${missing[*]}
  Build it on the right machine (just release-artifacts $version) and upload (just release-upload $version)."
grep -qxF "mica-cli-$version-macos-arm64" <<<"$assets" \
  || note "note: no macOS CLI attached (not required; REQUIRE_MACOS=1 to insist)"

note "publishing $tag"
gh release edit "$tag" --repo "$GH_REPO" --draft=false --latest

latest=$(gh api "repos/$GH_REPO/releases/latest" --jq .tag_name)
[ "$latest" = "$tag" ] \
  || fail "/releases/latest is $latest, expected $tag — the in-app updater will not see this release"
note "$tag is public and latest"
