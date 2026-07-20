#!/usr/bin/env bash
# Node-side deploy entry point. Installed at /usr/local/sbin/mica-deploy,
# owned root:root 0755 (the deploy account must NOT be able to rewrite it).
#
# This is the ONLY thing the CI key can run. `~mica-deploy/.ssh/authorized_keys`
# pins it with `restrict,command="/usr/bin/sudo /usr/local/sbin/mica-deploy"`,
# which sshd evaluates BEFORE accepting the key: whatever the client asks for
# lands in SSH_ORIGINAL_COMMAND and is validated here, and nothing else can run.
# So a leaked CI key cannot open a shell, read .env (JWT_SECRET, DB password,
# OSS keys), pg_dump the database, or install anything — it can only move prod
# to a version that already exists in the registry.
#
# That is the point. `docs/release.md` used to argue CI must not deploy because
# it would need a prod root key; the premise is wrong — an SSH key's authority
# is whatever `command=` leaves it, not root by default.
#
# Usage (as the CI key does it):
#   ssh mica-deploy@<node> "deploy 0.12.8"
# Still runnable by hand as root:
#   /usr/local/sbin/mica-deploy deploy 0.12.8
set -euo pipefail

NODE_DIR=/data/mica

fail() { printf 'mica-deploy REFUSED: %s\n' "$*" >&2; exit 1; }

# Prefer the forced-command payload; fall back to argv for a manual root run.
raw="${SSH_ORIGINAL_COMMAND:-$*}"
[[ "$raw" != *$'\n'* ]] || fail 'command must be a single line'

read -r action version compose_sha extra <<<"$raw"
[ -z "${extra:-}" ] || fail "unexpected extra arguments: $extra"
[ "${action:-}" = deploy ] || fail "only 'deploy' is permitted (got '${action:-}')"

# Immutable X.Y.Z only. This closes tag injection through SSH_ORIGINAL_COMMAND,
# and refuses rolling tags on principle: prod pins exact versions so a restart
# comes back on the same one.
[[ "${version:-}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
  || fail "version must be X.Y.Z (got '${version:-}')"
tag="v$version"

cd "$NODE_DIR" || fail "$NODE_DIR missing"

# Announce which build of THIS script is running. CI compares it against
# `git show <tag>:deploy/mica-deploy.sh | sha256sum` and fails the deploy on a
# mismatch — which turns "the node quietly runs an older policy than the repo
# describes" into a loud refusal.
#
# CI can only READ this. It must never be able to install the script: this file
# is the fence that limits the CI key, and anything that can rewrite the fence
# is not limited by it. Installing stays a root-side, out-of-band act
# (`just sync-deploy-script`).
echo "script_sha=$(sha256sum "$0" | cut -c1-16)"

# Optional compose fingerprint. CI passes the sha256 of deploy/docker-compose.yml
# AT THE TAG; if the node's file differs, refuse rather than deploy a version
# against a compose it was never released with.
#
# A hash grants NO authority — the worst an attacker can do with it is pass the
# right one (an ordinary deploy) or a wrong one (a refused deploy). That is why
# the fingerprint is checked here instead of letting the caller supply the file:
# supplying compose would mean supplying `-v /:/host`, i.e. host root.
if [ -n "${compose_sha:-}" ]; then
  [[ "$compose_sha" =~ ^[0-9a-f]{64}$ ]] || fail 'compose sha must be 64 hex chars'
  actual=$(sha256sum docker-compose.yaml | cut -d' ' -f1)
  [ "$actual" = "$compose_sha" ] || fail \
    "compose on this node does not match $tag (node=${actual:0:16}… want=${compose_sha:0:16}…) — run 'just deploy-prod $version' from a machine that can reach GitHub to sync it"
fi

# --- this script does NOT touch docker-compose.yaml ------------------------
# It moves a version, nothing else. Two reasons, and the second is why the first
# is affordable:
#
#  1. SECURITY. The whole value of the forced command is that a leaked CI key
#     can only select an already-published version. Let it supply compose and it
#     can mount the host root into a container — that is host root, not merely a
#     compromised container — and no validation reliably prevents it (pid: host,
#     cap_add, devices, the docker socket… a denylist loses that game).
#  2. The obvious way to pair compose WITH the tag does not work on this node.
#     It cannot reach github.com at all — 3/3 timeouts at 15 s, while ACR answers
#     in 0.14 s — so fetching the tag is unavailable, and a `--filter=blob:none`
#     clone makes it worse (the ref resolves, then the blob fetch hangs).
#     Shipping compose as an OCI artifact through ACR was the next idea; Aliyun
#     ACR rejects it with `blob type invalid` on a custom media type. (Docker
#     pulls artifacts fine — that was never the blocker.)
#
# So compose stays node-managed, synced by `just deploy-prod` from a machine that
# CAN reach GitHub, where it is already taken from the tag rather than from the
# working tree. Compose changes rarely; versions change every release.
prev=$(sed -nE 's|^MICA_VERSION=(.*)$|\1|p' .env)
[ -n "$prev" ] || fail '.env has no MICA_VERSION to roll back to'

# Restore MICA_VERSION on ANY non-zero exit, not just a failed health check.
#
# Without this trap, `set -e` aborted the moment `docker compose pull` failed —
# after .env had already been rewritten — and the rollback at the bottom never
# ran. Deploying a version that does not exist in the registry left .env
# pointing at it: the running containers were untouched (so nothing looked
# wrong), but the persisted desired state was broken and the next restart or
# reboot would have tried to start a tag that isn't there. Observed exactly that
# with `deploy 9.9.9`.
rollback() {
  local rc=$?
  [ $rc -eq 0 ] && return 0
  local now
  now=$(sed -nE 's|^MICA_VERSION=(.*)$|\1|p' .env)
  if [ "$now" != "$prev" ]; then
    echo "==> failed (rc=$rc); restoring MICA_VERSION=$prev" >&2
    sed -i -E "s|^MICA_VERSION=.*|MICA_VERSION=$prev|" .env
    # Best-effort: bring the previous version back up. Its images are already
    # local, so this is fast and needs no registry.
    docker compose up -d --no-deps api web >&2 || true
  fi
  return $rc
}
trap rollback EXIT

sed -i -E "s|^MICA_VERSION=.*|MICA_VERSION=$tag|" .env

echo "==> $prev -> $tag"
docker compose pull api web
docker compose up -d --no-deps api web

# The backup sidecar shares MICA_VERSION, so it drifts unless it moves too — it
# once sat on willdockerhub/mica-cli:v0.3 for many releases. Only refresh it
# where it is ALREADY running, so a deploy never switches backups on.
if [ -n "$(docker compose --profile backup ps -aq backup 2>/dev/null)" ]; then
  docker compose --profile backup pull backup
  docker compose --profile backup up -d --no-deps backup
  echo 'backup sidecar refreshed'
fi

for _ in $(seq 1 60); do
  state=$(docker inspect --format '{{.State.Health.Status}}' mica-api-1 2>/dev/null || true)
  if [ "$state" = healthy ]; then
    echo "deployed=$tag healthy=yes"
    exit 0
  fi
  sleep 4
done

# Health never came up. The EXIT trap does the actual restoring — this just
# reports why.
#
# NOTE: rollback covers the VERSION ONLY. `sqlx::migrate!` is forward-only, so
# if the failed version ran a migration the schema stays migrated and the old
# api meets a newer schema. That is exactly why release.md requires a pg_dump
# restore point before any data-affecting release — this cannot replace it.
fail "api not healthy after 4 min (schema NOT rolled back — see release.md)"
