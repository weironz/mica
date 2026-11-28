#!/usr/bin/env bash
# The whole release train on THIS machine — the FALLBACK for when GitHub
# Actions cannot run. When it can, the hosted Release workflow is the default
# (docs/ops/release.md): one-off runners, parallel builds, no local state.
#
#   just ship 0.13.57
#
#   0. release-preflight            gh API, Inno, Docker + logins, disk, DB… all at once
#   1. just release X.Y.Z           bump → gate (Rust workspace + mica_rust + wasm32 + clippy) → commit → tag
#   2. git push origin main vX.Y.Z
#   3. just release-artifacts       what this host can build (see that script)
#   4. just release-images          if Docker is up (skip: IMAGES=0)
#   5. just release-upload          draft release + assets
#   6. just release-publish         only if every required asset + sha256 manifest match
#      (Note: Playwright Web E2E requires Linux/CI; Windows hosts skip it locally.)
#
# Re-running after a failure resumes from the tag: an artifacts build or image
# push already completed for this exact commit is reused, not redone
# (REBUILD=1 to force). Every stage is timed; the summary prints even on failure.
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
out=$(dist_dir "$version")
os=$(host_os)
require_gh

timings=()
current=""
started=$SECONDS
fmt() { printf '%dm%02ds' $(($1 / 60)) $(($1 % 60)); }
summary() {
  local status=$?
  printf '\n  ship %s — stage times\n' "$tag"
  for t in "${timings[@]}"; do printf '    %s\n' "$t"; done
  [ -n "$current" ] && printf '    %-22s FAILED after %s\n' "$current" "$(fmt $((SECONDS - stage_t0)))"
  printf '    %-22s %s\n' total "$(fmt $((SECONDS - started)))"
  return $status
}
trap summary EXIT
stage() { # $1 = name, rest = command
  current=$1; shift
  stage_t0=$SECONDS
  note "[$current] $(date +%H:%M:%S)"
  "$@"
  timings+=("$(printf '%-22s %s' "$current" "$(fmt $((SECONDS - stage_t0)))")")
  current=""
}
skip() { timings+=("$(printf '%-22s reused (%s)' "$1" "$2")"); note "[$1] reused: $2"; }

stage preflight bash scripts/release-preflight.sh "$version"

if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  note "tag $tag exists locally — resuming after the release commit"
  [ "$(git rev-parse HEAD)" = "$(git rev-parse "$tag^{commit}")" ] || git checkout -q "$tag"
else
  stage release bash scripts/release.sh "$version"
fi
sha=$(git rev-parse HEAD)

stage push git push origin "HEAD:main" "$tag"

if [ "${REBUILD:-}" != 1 ] && [ "$(cat "$out/.built-from" 2>/dev/null)" = "$sha" ]; then
  skip artifacts "$out already built from ${sha:0:12}"
else
  stage artifacts bash scripts/release-artifacts.sh "$version"
fi

if [ "${IMAGES:-1}" != 1 ] || ! docker info >/dev/null 2>&1; then
  note "images skipped (Docker down or IMAGES=0) — run 'just release-images $version' where Docker is"
elif [ "${REBUILD:-}" != 1 ] && [ "$(cat "$out/.images-pushed" 2>/dev/null)" = "$sha" ]; then
  skip images "already pushed from ${sha:0:12}"
else
  stage images bash scripts/release-images.sh "$version"
fi

stage upload bash scripts/release-upload.sh "$version"

# Not under `if`: errexit is off inside an `if` condition, so a refused publish
# would count as a passed stage. A refusal exits here and the summary says so;
# release-publish itself names what is missing.
stage publish bash scripts/release-publish.sh "$version"
cat <<EOF

  $tag is published. Going live is the next, separate decision:
    just deploy-prod $version --check --diff     # rehearse against the real node
    just deploy-prod $version
EOF
