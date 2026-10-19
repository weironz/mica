#!/usr/bin/env bash
# The whole release train on THIS machine, for when GitHub Actions cannot run:
#
#   just ship 0.13.57
#
#   1. just release X.Y.Z          bump → gate (tests, clippy, flutter) → commit → tag
#   2. git push origin main vX.Y.Z
#   3. just release-artifacts      what this host can build (see that script)
#   4. just release-images         if Docker is up (skip: IMAGES=0)
#   5. just release-upload         draft release + assets
#   6. just release-publish        only if every required asset is attached;
#                                  otherwise it says which machine still has to
#                                  run steps 3 + 5
#
# Deploying stays a separate, explicit step — building is a consequence of
# tagging, going live is a decision (docs/ops/release.md):
#   just deploy-prod X.Y.Z --check --diff     (rehearse)
#   just deploy-prod X.Y.Z
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=${1:-}
check_version_arg "$version"
tag="v$version"
require_gh

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  note "tag $tag exists locally — resuming after the release commit"
  [ "$(git rev-parse HEAD)" = "$(git rev-parse "$tag^{commit}")" ] || git checkout -q "$tag"
else
  bash scripts/release.sh "$version"
fi

note "push main + $tag"
git push origin "HEAD:main" "$tag"

bash scripts/release-artifacts.sh "$version"

if [ "${IMAGES:-1}" = 1 ] && docker info >/dev/null 2>&1; then
  bash scripts/release-images.sh "$version"
else
  note "images skipped (Docker down or IMAGES=0) — run 'just release-images $version' where Docker is"
fi

bash scripts/release-upload.sh "$version"

if bash scripts/release-publish.sh "$version"; then
  cat <<EOF

  $tag is published. Going live is the next, separate decision:
    just deploy-prod $version --check --diff     # rehearse against the real node
    just deploy-prod $version
EOF
else
  cat <<EOF

  $tag is uploaded as a DRAFT but not published — see what is missing above.
  On the machine that can build it:
    git fetch --tags && git checkout $tag
    just release-artifacts $version && just release-upload $version
  then, anywhere:
    just release-publish $version
EOF
fi
