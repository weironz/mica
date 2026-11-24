#!/usr/bin/env bash
# Print affected CI jobs for base..head. No arguments means full CI.
# --stdin reads NUL-separated paths for fixture tests.
set -euo pipefail

all_jobs() { printf '%s\n' rust client container mcp; }

if [ "$#" -eq 0 ]; then
  all_jobs
  exit 0
fi

rust=false client=false container=false mcp=false

classify() {
  while IFS= read -r -d '' path; do
    case "$path" in
      docs/*|README*|CHANGELOG*|LICENSE*|AGENTS.md|CLAUDE.md)
        ;;
      crates/md-core/*|crates/md-wasm/*|crates/mica-core/*|crates/mica-board/*|crates/mica-sync/*|crates/mica-editor/*|crates/mica-client/*|crates/api-types/*)
        rust=true client=true container=true mcp=true
        ;;
      crates/*|migrations/*)
        rust=true container=true mcp=true
        ;;
      clients/mica_rust/*)
        client=true container=true
        ;;
      deploy/*|e2e/*)
        container=true
        ;;
      *)
        rust=true client=true container=true mcp=true
        ;;
    esac
  done
}

if [ "$#" -eq 1 ] && [ "$1" = --stdin ]; then
  classify
elif [ "$#" -eq 2 ]; then
  paths=$(mktemp)
  trap 'rm -f "$paths"' EXIT
  git diff --name-only --no-renames -z "$1" "$2" -- > "$paths"
  classify < "$paths"
else
  echo 'usage: ci-paths-for-scope.sh [base head | --stdin]' >&2
  exit 2
fi

for job in rust client container mcp; do
  if [ "${!job}" = true ]; then
    echo "$job"
  fi
done
