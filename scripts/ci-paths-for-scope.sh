#!/usr/bin/env bash
# Print affected jobs for base..head. No arguments means full CI.
# --stdin reads NUL-separated paths for fixture tests.
set -euo pipefail

all_jobs() { printf '%s\n' rust flutter web-e2e container mcp; }

if [ "$#" -eq 0 ]; then
  all_jobs
  exit 0
fi

rust=false flutter=false web=false container=false mcp=false

classify() {
  while IFS= read -r -d '' path; do
    case "$path" in
      docs/*|README*|CHANGELOG*|LICENSE*|AGENTS.md|CLAUDE.md)
        ;;
      crates/*|migrations/*)
        rust=true web=true container=true mcp=true
        ;;
      clients/mica_flutter/rust/*)
        rust=true flutter=true web=true container=true mcp=true
        ;;
      clients/mica_flutter/*|.fvmrc)
        flutter=true web=true
        ;;
      e2e/docker-compose*|e2e/.env.ci|deploy/nginx*)
        web=true container=true
        ;;
      e2e/*)
        web=true
        ;;
      deploy/Dockerfile*|deploy/docker-compose*)
        container=true
        ;;
      *)
        # CI/toolchain changes and unclassified paths fail closed.
        rust=true flutter=true web=true container=true mcp=true
        ;;
    esac
  done
}

if [ "$#" -eq 1 ] && [ "$1" = --stdin ]; then
  classify
elif [ "$#" -eq 2 ]; then
  paths=$(mktemp)
  trap 'rm -f "$paths"' EXIT
  # Disable rename detection so both the old and new paths are classified.
  # Do not hide git failures inside a pipe or a process substitution.
  git diff --name-only --no-renames -z "$1" "$2" -- > "$paths"
  classify < "$paths"
else
  echo 'usage: ci-paths-for-scope.sh [base head | --stdin]' >&2
  exit 2
fi

for job in rust flutter web container mcp; do
  if [ "${!job}" = true ]; then
    case "$job" in web) echo web-e2e ;; *) echo "$job" ;; esac
  fi
done
