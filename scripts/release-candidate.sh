#!/usr/bin/env bash
# Prepare and promote the exact commit tested by the hosted Ship workflow.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

remote_sha() {
  local result
  result=$(git ls-remote origin "$1") || return
  printf '%s' "${result%%$'\t'*}"
}

prepare() {
  local current version base candidate latest
  [[ "$GITHUB_REF" == refs/heads/* ]] || fail "Ship must be dispatched on a branch"
  base=$(git rev-parse HEAD)
  current=$(bash scripts/manifest-version.sh)
  version=${SHIP_VERSION:-}
  if [ -z "$version" ]; then
    local patch=${current##*.}
    version="${current%.*}.$((10#$patch + 1))"
  fi
  [[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] \
    || fail "release version must be X.Y.Z with no leading zeros"
  [[ "$version" != "$current" && "$(printf '%s\n' "$current" "$version" | sort -V | tail -1)" == "$version" ]] \
    || fail "release version $version must be newer than $current"
  if [ "$SHIP_DRY_RUN" = false ]; then
    [ "$GITHUB_REF" = refs/heads/main ] || fail "production Ship must be dispatched on main"
    [ "$(remote_sha refs/heads/main)" = "$base" ] || fail "main has moved; start a new Ship from main"
    latest=$(gh api "repos/$GITHUB_REPOSITORY/releases/latest" --jq .tag_name)
    [ "$latest" = "v$current" ] \
      || fail "main version v$current differs from latest $latest; finish the previous release before starting another"
  fi
  local existing_tag
  existing_tag=$(remote_sha "refs/tags/v$version")
  [ -z "$existing_tag" ] || fail "v$version already exists; use the previous run to continue"

  bash scripts/release-bump.sh "$version"
  bash scripts/release-tree-check.sh
  git -c user.name='github-actions[bot]' \
      -c user.email='41898282+github-actions[bot]@users.noreply.github.com' \
      commit -m "chore(release): $version" -- Cargo.toml Cargo.lock README.md \
      clients/mica_flutter/lib/main.dart clients/mica_flutter/pubspec.yaml docs/ops/deploy.md
  [ -z "$(git status --porcelain)" ] || fail "version bump left other changes"
  candidate=$(git rev-parse HEAD)
  local ref="refs/heads/ship-candidates/$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT"
  git push --force-with-lease="$ref:" origin "$candidate:$ref"
  printf 'version=%s\nprevious_tag=v%s\nbase=%s\nsha=%s\nref=%s\n' \
    "$version" "$current" "$base" "$candidate" "$ref" >> "$GITHUB_OUTPUT"
}

promote() {
  local candidate=$1 base=$2 version=$3 tag="refs/tags/v$3"
  [[ "$candidate" =~ ^[0-9a-f]{40}$ && "$base" =~ ^[0-9a-f]{40}$ ]] || fail "invalid candidate/base SHA"
  [ "$(git rev-parse HEAD)" = "$candidate" ] || fail "checkout is not the CI candidate"
  [ "$(git rev-list --parents -n 1 "$candidate")" = "$candidate $base" ] \
    || fail "candidate must be the direct child of the selected main commit"
  bash scripts/release-version.sh "$version"

  local live tag_sha
  live=$(remote_sha refs/heads/main)
  tag_sha=$(remote_sha "$tag")
  # An interrupted runner may have succeeded at the atomic push already.
  if [ "$live" = "$candidate" ] && [ "$tag_sha" = "$candidate" ]; then
    note "main and v$version already point at the tested commit"
    return
  fi
  [ "$live" = "$base" ] || fail "main has moved while CI ran; start a new Ship from main"
  [ -z "$tag_sha" ] || fail "v$version already exists; refusing to move it"
  git push --atomic --force-with-lease="refs/heads/main:$base" \
    --force-with-lease="$tag:" origin "$candidate:refs/heads/main" "$candidate:$tag"
}

case "${1:-}" in
  prepare) prepare ;;
  promote) promote "$2" "$3" "$4" ;;
  *) fail "usage: release-candidate.sh prepare | promote <candidate> <base> <version>" ;;
esac
