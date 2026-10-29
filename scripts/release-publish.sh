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

assets_tsv=$(gh release view "$tag" --repo "$GH_REPO" --json assets \
  --jq '.assets[] | [.name, (.digest // "")] | @tsv') \
  || fail "no release $tag — run 'just release-upload $version' first"
assets=$(cut -f1 <<<"$assets_tsv")

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

# An asset's checksum manifest is not enough: the manifest must actually cover
# the asset, and its hash must be the digest of the asset GitHub holds. A
# manifest that merely exists could be stale or hand-edited, and the updater
# downloads sha256-<os>.txt and refuses a binary whose line is missing or wrong
# (crates/cli/src/update.rs) — so a release that fails this is un-installable on
# that platform BY CONSTRUCTION, and nothing downstream re-checks it (#113).
dl=$(mktemp -d)
trap 'rm -rf -- "$dl"' EXIT

digest_of() { # $1 = asset name -> sha256 hex GitHub recorded, or empty
  # gh reports digests as `sha256:<hex>` (it changed from bare hex); the
  # manifests carry the bare hex. Comparing the prefixed string reported a
  # stale manifest when both sides were byte-identical (v0.13.69).
  awk -F'\t' -v n="$1" '$1 == n { print $2 }' <<<"$assets_tsv" \
    | sed 's/^sha256://'
}

verify_asset() { # $1 = asset name
  local asset=$1 m mf listed want actual
  m=$(manifest_for_asset "$asset") \
    || fail "no checksum manifest is known for $asset — teach manifest_for_asset (scripts/lib-local.sh) about it"
  grep -qxF "$m" <<<"$assets" \
    || fail "missing checksum manifest(s): $m
  The attached assets cannot be verified without it, so users on that platform
  would be offered an update the CLI refuses to install. Build + upload on the
  matching machine (just release-artifacts $version) — release-artifacts writes
  one manifest per platform it produced, including a cross-built one."
  gh release download "$tag" --repo "$GH_REPO" --pattern "$m" --dir "$dl" --clobber >/dev/null
  mf="$dl/$m"
  listed=$(awk -v n="$asset" '$2 == n { print $1 }' "$mf")
  [ -n "$listed" ] \
    || fail "$m does not list $asset — the manifest names the wrong files and the updater would refuse this asset"
  want=$(digest_of "$asset")
  if [ -n "$want" ]; then
    [ "$listed" = "$want" ] \
      || fail "$asset in $m hashes to $listed but GitHub recorded $want — the manifest is stale or the asset was replaced; re-upload both"
  else
    # No server-side digest (old upload path): hash the attached asset itself.
    gh release download "$tag" --repo "$GH_REPO" --pattern "$asset" --dir "$dl" --clobber >/dev/null
    actual=$(sha256sum "$dl/$asset" | cut -d' ' -f1)
    [ "$listed" = "$actual" ] \
      || fail "$asset in $m hashes to $listed but the attached file is $actual — the manifest is stale; re-upload both"
  fi
}

for a in "${required[@]}"; do
  verify_asset "$a"
done
# An attached macOS CLI was never required, but once it IS attached it becomes
# downloadable and must be verifiable like every other asset.
if grep -qxF "mica-cli-$version-macos-arm64" <<<"$assets"; then
  verify_asset "mica-cli-$version-macos-arm64"
else
  note "note: no macOS CLI attached (not required; REQUIRE_MACOS=1 to insist)"
fi

note "publishing $tag"
gh release edit "$tag" --repo "$GH_REPO" --draft=false --latest

latest=$(gh api "repos/$GH_REPO/releases/latest" --jq .tag_name)
[ "$latest" = "$tag" ] \
  || fail "/releases/latest is $latest, expected $tag — the in-app updater will not see this release"
note "$tag is public and latest"
