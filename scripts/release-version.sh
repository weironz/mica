#!/usr/bin/env bash
# Resolve the version a release run is FOR, and REFUSE unless this tree agrees.
#
#   bash scripts/release-version.sh 0.13.69     # explicit (workflow_dispatch)
#   GITHUB_REF_NAME=v0.13.69 bash scripts/release-version.sh
#
# The single version resolver for .github/workflows/release.yml. `draft` runs it
# once and exports the answer as its job outputs; `windows`, `cli` and `images`
# declare `needs: draft` and read those outputs, so no leg can tag an artifact
# with a version another leg never saw, and a refusal here skips every build and
# push (#119). The images leg used to resolve the version for itself and had no
# gate at all, so a draft refusal ("asked for 0.13.69, this tree says 0.13.68")
# still pushed 0.13.69 image tags under publish=true.
#
# The tag is not evidence of anything by itself: `git tag v0.13.29 && git push`
# without `just release` (which is what bumps) builds an installer NAMED 0.13.29
# around a binary whose kAppVersion is still 0.13.28, and the in-app updater
# compares exactly that number. Asserted on the dispatch path too: a manual run
# builds from the default branch, and an input that disagrees with that tree
# mislabels the artifact in exactly the same way.
set -euo pipefail

explicit=${1:-}
if [ -n "$explicit" ]; then
  version=$explicit
else
  ref=${GITHUB_REF_NAME:-}
  [ -n "$ref" ] || {
    echo "::error::no version given and GITHUB_REF_NAME is empty — pass an explicit X.Y.Z or push a v* tag" >&2
    exit 1
  }
  version=${ref#v}
fi

# Exactly X.Y.Z: three non-empty numeric parts, no other characters, no leading
# `v`. Anything else is an input mistake, not a version to build a release for.
case "$version" in
  '' | *[!0-9.]* | .* | *. | *..*) bad=1 ;;
  *) bad=0 ;;
esac
major=${version%%.*}
minor_patch=${version#*.}
minor=${minor_patch%%.*}
patch=${minor_patch#*.}
case "$patch" in
  *.*) bad=1 ;;
esac
[ -n "$major" ] && [ -n "$minor" ] && [ -n "$patch" ] || bad=1
if [ "$bad" = 1 ]; then
  echo "::error::'$version' is not X.Y.Z (three dot-separated numbers, no leading v)" >&2
  exit 1
fi

tree=$(bash scripts/manifest-version.sh) || exit 1
[ "$tree" = "$version" ] || {
  echo "::error::asked for $version, but this tree says $tree." \
       "Was it tagged by hand instead of with 'just release $version'?" >&2
  exit 1
}

printf '%s\n' "$version"