#!/usr/bin/env bash
# Select a meaningful diff base for the structure gate.
#
# CI is workflow_dispatch-only since 2026-10-20 (Actions quota, user's call), so
# there is no push event to read `before` from — that branch was unreachable and
# is gone (#88). A pull_request trigger, if ever re-enabled, still gets its
# merge-base. On main itself a merge-base against origin/main would name HEAD
# and silently skip every growth check, which is why the fallback is HEAD^.
set -euo pipefail
cd "$(dirname "$0")/.."

valid_commit() {
  [[ -n "$1" && ! "$1" =~ ^0+$ ]] && git cat-file -e "$1^{commit}" 2>/dev/null
}

base=
if [ "${GITHUB_EVENT_NAME:-workflow_dispatch}" = pull_request ]; then
  pr_base=${STRUCTURE_PR_BASE:-}
  if valid_commit "$pr_base"; then
    base=$(git merge-base "$pr_base" HEAD)
  else
    echo 'WARN  structure: PR base unavailable; falling back to HEAD^' >&2
  fi
fi

if [[ -z "$base" ]] && valid_commit HEAD^; then
  # A manual dispatch cannot reconstruct multi-commit growth since the last
  # run: this fallback checks the latest commit and keeps the absolute/ratchet
  # checks in every case.
  base=$(git rev-parse HEAD^)
fi
if [[ -n "$base" ]]; then
  echo "structure: diff base $base"
  bash scripts/check-file-size.sh --diff "$base"
else
  echo 'WARN  structure: no parent history; running absolute and ratchet checks only' >&2
  bash scripts/check-file-size.sh
fi
