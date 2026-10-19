#!/usr/bin/env bash
# Attach dist/vX.Y.Z/* to the GitHub release vX.Y.Z, creating it as a DRAFT
# if it does not exist yet.
#
#   just release-upload 0.13.57
#
# Safe to run from several machines (Windows box for the installer, a Mac for
# the macOS CLI, …): each attaches what it built, `--clobber` replaces a file
# re-uploaded under the same name. Nothing becomes public here — the release
# stays a draft until `just release-publish`, which is the same guarantee the
# draft step in release.yml gives the in-app updater: /releases/latest never
# points at a release whose installer is not attached yet.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=${1:-}
check_version_arg "$version"
tag="v$version"
out=$(dist_dir "$version")
require_gh

compgen -G "$out/*" >/dev/null || fail "$out is empty — run 'just release-artifacts $version' first"

git ls-remote --exit-code --tags origin "$tag" >/dev/null 2>&1 \
  || fail "tag $tag is not on origin — push it first: git push origin main $tag"

if ! gh release view "$tag" --repo "$GH_REPO" >/dev/null 2>&1; then
  note "creating draft release $tag"
  prev=$(git describe --tags --abbrev=0 "$tag^" 2>/dev/null || true)
  notes=$(mktemp)
  trap 'rm -f "$notes"' EXIT
  {
    if [ -n "$prev" ]; then
      echo "## Changes since $prev"
      echo
      git log --no-merges --format='- %s' "$prev..$tag"
    else
      echo "Release $tag"
    fi
  } > "$notes"
  gh release create "$tag" --repo "$GH_REPO" --draft --verify-tag \
    --title "$tag" --notes-file "$notes"
else
  draft=$(gh release view "$tag" --repo "$GH_REPO" --json isDraft --jq .isDraft)
  [ "$draft" = true ] || [ "${FORCE:-}" = 1 ] \
    || fail "$tag is already PUBLISHED — replacing its files changes what users already downloaded. FORCE=1 to do it anyway."
fi

note "uploading $(ls "$out" | wc -l | tr -d ' ') file(s) from $out"
gh release upload "$tag" "$out"/* --repo "$GH_REPO" --clobber
gh release view "$tag" --repo "$GH_REPO" --json assets --jq '.assets[].name' | sed 's/^/    /'
