#!/usr/bin/env bash
# Everything the local release train will need, checked BEFORE anything is
# bumped, tagged or built — and reported all at once, not one per attempt.
#
#   just release-preflight 0.14.9
#   IMAGES=0 just release-preflight 0.14.9     # not pushing images from here
#
# 0.14.8's local train took three hours, most of it re-runs: Inno Setup turned
# out to be missing only after the Flutter build, `gh` passed `auth status` but
# could not reach the API from a background shell, and so on — each found one
# at a time, deep into the run. This answers all of them in seconds. `just ship`
# runs it first; a resume (the tag already exists) skips the checks that only
# matter before tagging.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=${1:-}
check_version_arg "$version"
tag="v$version"
os=$(host_os)
problems=()
problem() { problems+=("$*"); }
ok() { note "ok: $*"; }

resume=0
git rev-parse -q --verify "refs/tags/$tag" >/dev/null && resume=1

# GitHub: the real API, not the cached login (see require_gh_api).
if (require_gh_api) >/dev/null 2>&1; then ok "gh reaches the GitHub API"; else
  problem "gh cannot call the GitHub API from this shell — 'gh api repos/$GH_REPO' must work (gh auth login; in a background session, check the keyring)"
fi

if [ "$resume" = 0 ]; then
  if git diff --quiet && git diff --cached --quiet; then ok "working tree clean"; else
    problem "working tree is dirty — commit or stash first (just release refuses too, but only after you waited for it)"
  fi
  if git fetch -q origin main 2>/dev/null; then
    if git merge-base --is-ancestor origin/main HEAD; then ok "HEAD contains origin/main"; else
      problem "HEAD does not contain origin/main — the push of main would be rejected after the gate ran. Pull first."
    fi
  else
    problem "cannot fetch origin/main"
  fi
  if git ls-remote --exit-code --tags origin "$tag" >/dev/null 2>&1; then
    problem "$tag already exists on origin — pick the next version"
  else
    ok "$tag is free on origin"
  fi
  if [ -n "$(resolve_database_url)" ]; then ok "test Postgres reachable"; else
    problem "no reachable Postgres for the DB-backed tests — 'just dev' (or export DATABASE_URL); release-check refuses without it"
  fi
else
  ok "$tag exists locally — resuming, pre-tag checks skipped"
fi

# Disk: the gate's Rust builds plus three images need room.
min_gb=${MICA_MIN_FREE_GB:-20}
free_kb=$(df -Pk . | awk 'NR == 2 { print $4 }')
if [ -n "$free_kb" ] && [ "$free_kb" -ge $((min_gb * 1024 * 1024)) ]; then
  ok "$((free_kb / 1024 / 1024)) GB free"
else
  problem "only $(( ${free_kb:-0} / 1024 / 1024 )) GB free here; a release needs about ${min_gb} GB (MICA_MIN_FREE_GB)"
fi

case "$os" in
  windows)
    if find_iscc >/dev/null; then ok "Inno Setup found"; else
      problem "Inno Setup 6 (ISCC.exe) not found — install it or set ISCC; the installer build needs it"
    fi
    ;;
  linux)
    command -v musl-gcc >/dev/null 2>&1 || docker info >/dev/null 2>&1 \
      || problem "neither musl-tools nor Docker is available for the static Linux CLI"
    ;;
esac

docker_needed=0
[ "${IMAGES:-1}" = 1 ] && docker_needed=1
[ "$os" = windows ] && docker_needed=1 # the Linux CLI is cross-built in Docker
if [ "$docker_needed" = 1 ]; then
  if docker info >/dev/null 2>&1; then
    ok "Docker is up"
    if [ "${IMAGES:-1}" = 1 ]; then
      config="${DOCKER_CONFIG:-$HOME/.docker}/config.json"
      for registry in registry.cn-shenzhen.aliyuncs.com index.docker.io; do
        if [ -f "$config" ] && grep -q "$registry" "$config"; then ok "docker login: $registry"; else
          problem "no docker login for $registry — 'docker login ${registry/index.docker.io/}' (images push to both)"
        fi
      done
    fi
  else
    problem "Docker is not running — needed for the images and (on Windows) the Linux CLI. Start it, or IMAGES=0 and build images elsewhere."
  fi
fi

if [ ${#problems[@]} -gt 0 ]; then
  printf '\n  REFUSED: %d preflight problem(s):\n' "${#problems[@]}" >&2
  for p in "${problems[@]}"; do printf '    - %s\n' "$p" >&2; done
  exit 1
fi
printf '\n  release preflight passed for %s\n' "$version"
