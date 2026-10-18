#!/usr/bin/env bash
# Select a meaningful diff base for the structure gate. On push, origin/main
# already names HEAD; using its merge-base would silently skip every growth check.
set -euo pipefail
cd "$(dirname "$0")/.."

valid_commit() {
  [[ -n "$1" && ! "$1" =~ ^0+$ ]] && git cat-file -e "$1^{commit}" 2>/dev/null
}

base=
case "${GITHUB_EVENT_NAME:-workflow_dispatch}" in
  push)
    before=${STRUCTURE_PUSH_BEFORE:-}
    if valid_commit "$before"; then
      base=$before
    else
      echo 'WARN  structure: push.before unavailable (including initial push); falling back to HEAD^' >&2
    fi
    ;;
  pull_request)
    pr_base=${STRUCTURE_PR_BASE:-}
    if valid_commit "$pr_base"; then
      base=$(git merge-base "$pr_base" HEAD)
    else
      echo 'WARN  structure: PR base unavailable; falling back to HEAD^' >&2
    fi
    ;;
esac

if [[ -z "$base" ]] && valid_commit HEAD^; then
  # A missing event SHA cannot reconstruct a multi-commit push: this fallback
  # checks the latest commit and keeps the absolute/ratchet checks in every case.
  base=$(git rev-parse HEAD^)
fi
if [[ -n "$base" ]]; then
  echo "structure: diff base $base"
  bash scripts/check-file-size.sh --diff "$base"
else
  echo 'WARN  structure: no parent history; running absolute and ratchet checks only' >&2
  bash scripts/check-file-size.sh
fi
