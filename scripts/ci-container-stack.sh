#!/usr/bin/env bash
# The `container` job's Compose stack, as three commands: up | logs | down.
#
# Every command runs under ONE explicit, per-run project name. That is not
# cosmetic: without `--project-name` compose derives the project from the
# checkout directory — which is `mica` here, exactly the name a developer's
# `just dev` stack on the same Docker daemon uses. `down -v --remove-orphans`
# would then delete THEIR containers, network and named volumes. (`-v` is
# required: the postgres/rustfs volumes are what make the stack a stack, and
# leaving them behind poisons the next run's data.)
#
# GITHUB_RUN_ID + GITHUB_RUN_ATTEMPT identify one attempt and are identical in
# every step of it, so up/logs/down meet on the same project without sharing
# shell state. Locally both are unset and the name is a stable `mica-ci-local-0`
# — still not `mica`.
#
# CONCURRENCY: deploy/docker-compose.single.yml hardcodes host ports 80/9000 and
# ${RUSTFS_CONSOLE_PORT:-9001}, and declares no `container_name:`. So two stacks
# can share a daemon only if those ports differ; with the defaults they cannot.
# A self-hosted runner executes one job at a time, which is what keeps this
# correct; the per-run project name is what keeps a stale run's stack, or a
# developer's, out of reach either way.
set -euo pipefail

project=${MICA_CI_COMPOSE_PROJECT:-mica-ci-${GITHUB_RUN_ID:-local}-${GITHUB_RUN_ATTEMPT:-0}}

# The developer stack's implicit project name (the directory compose runs in).
# Refused loudly rather than trusted: this script exists because `down -v` on
# the wrong project is destructive and silent.
[ "$project" != "mica" ] \
  || { echo "REFUSED: project 'mica' is the developer stack's default project — set MICA_CI_COMPOSE_PROJECT to something run-scoped" >&2; exit 1; }

compose() {
  docker compose --project-directory . --project-name "$project" \
    --env-file e2e/.env.ci \
    -f deploy/docker-compose.single.yml \
    -f e2e/docker-compose.from-source.yml "$@"
}

case "${1:-}" in
  up)
    # --renew-anon-volumes --force-recreate: start from a clean state instead of
    # inheriting whatever a previous attempt left behind.
    # --wait blocks until every service with a healthcheck reports healthy, so a
    # stack that never comes up fails HERE, with compose naming the service.
    compose up -d --renew-anon-volumes --force-recreate --wait --wait-timeout 300
    ;;
  logs)
    compose logs --tail "${2:-120}"
    ;;
  down)
    # -v drops the named volumes; --remove-orphans anything a service rename or
    # removal left in the project. Always scoped to $project, never the default.
    compose down -v --remove-orphans
    ;;
  *)
    echo "usage: $0 up|logs|down  (project: $project)" >&2
    exit 2
    ;;
esac