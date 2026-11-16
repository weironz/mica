#!/usr/bin/env bash
# The whole release train on THIS machine — the FALLBACK for when GitHub
# Actions cannot run. When it can, the hosted Release workflow is the default
# (docs/ops/release.md): one-off runners, parallel builds, no local state.
#
#   just ship 0.13.57
#
#   0. release-preflight            gh API, Inno, Docker + logins, disk, DB… all at once
#   1. just release X.Y.Z           bump → gate (tests, clippy, flutter) → commit → tag
#   2. git push origin main vX.Y.Z
#      + browser-only suites        Windows cannot run them (#181): dispatched to
#                                   Actions now, waited for before step 6
#   3. just release-artifacts       what this host can build (see that script)
#   4. just release-images          if Docker is up (skip: IMAGES=0)
#   5. just release-upload          draft release + assets
#   6. just release-publish         only if every required asset is attached AND
#                                   the browser suites passed on the tag commit
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

# Browser-only suites: release acceptance, and the one gate this host may not
# be able to run itself. Started NOW so it runs while the installer builds.
browser_state=$(browser_status "$sha")
if [ "$browser_state" = success ]; then
  note "browser suites already passed on ${sha:0:12}"
elif [ "$os" != windows ] && [ -z "${MICA_BROWSER_HOSTED:-}" ]; then
  stage browser-tests bash scripts/browser-tests.sh --report-status
elif [ "$browser_state" = pending ]; then
  note "browser suites already running for ${sha:0:12}"
else
  note "dispatching the browser suites to GitHub Actions for $tag (Windows cannot run them, #181)"
  gh workflow run browser-tests.yml --repo "$GH_REPO" --ref main -f ref="$tag"
fi

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

# Wait for the browser status before publishing (release-publish requires it).
wait_browser() {
  local deadline=$((SECONDS + ${MICA_BROWSER_WAIT_MIN:-45} * 60)) state
  while :; do
    state=$(browser_status "$sha")
    case "$state" in
      success) note "browser suites passed on ${sha:0:12}"; return 0 ;;
      failure|error) fail "the browser suites FAILED on ${sha:0:12} — see the Browser tests run in Actions" ;;
    esac
    [ $SECONDS -lt $deadline ] || return 0 # release-publish refuses with the how-to
    note "waiting for the browser suites (status: ${state:-not started})…"
    sleep 30
  done
}
stage browser-wait wait_browser

# Not under `if`: errexit is off inside an `if` condition, so a refused publish
# would count as a passed stage. A refusal exits here and the summary says so;
# release-publish itself names what is missing.
stage publish bash scripts/release-publish.sh "$version"
cat <<EOF

  $tag is published. Going live is the next, separate decision:
    just deploy-prod $version --check --diff     # rehearse against the real node
    just deploy-prod $version
EOF
