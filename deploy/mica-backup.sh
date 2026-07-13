#!/usr/bin/env bash
# One scheduled Mica backup run, the EXTERNAL way (mechanism vs policy): export
# every workspace with the mica-cli thin API client, then let `rustic` (an
# external, restic-format tool) own encryption / dedup / retention. mica-cli
# itself has NO backup engine anymore — see docs/backup.md.
#
# Topology: ONE rustic repo (one bucket, one key, one dedup pool). Each workspace
# is its own retention lineage, keyed on the STABLE workspace id via rustic's
# `--label` (so a rename never splits history), with the mutable — and possibly
# non-ASCII — name carried as a readable `ws=<name>` tag. Retention groups by
# label, so `keep-daily/weekly/monthly` applies PER workspace.
#
# Config (all from env; secrets never hit the command line / `ps`):
#   MICA_SERVER            e.g. http://api:8080        (mica-cli export target)
#   MICA_TOKEN             a read-scoped Mica token
#   RUSTIC_PASSWORD        repo encryption password    (SAVE THIS OFF-HOST)
#   OSS_BUCKET OSS_ENDPOINT OSS_REGION OSS_ACCESS_KEY_ID OSS_SECRET_ACCESS_KEY
#   OSS_ROOT               opendal S3 backend (Aliyun OSS); root = key prefix
# Optional: MICA_EXPORT_DIR (default /var/lib/mica/export),
#           KEEP_DAILY/KEEP_WEEKLY/KEEP_MONTHLY (default 7/4/6),
#           MICA_CLI / RUSTIC (binary paths).
set -euo pipefail

MICA_CLI="${MICA_CLI:-mica-cli}"
# All rustic calls go through the wrapper, which renders /etc/rustic/rustic.toml
# from the OSS_* env before exec'ing rustic (see rustic-mica.sh).
RUSTIC="${RUSTIC:-rustic-mica}"
EXPORT_DIR="${MICA_EXPORT_DIR:-/var/lib/mica/export}"

log() { echo "[$(date -Is)] mica-backup: $*"; }

# 1) Guard: the repo must have been initialized once. We deliberately do NOT
#    auto-init in the loop — a misconfigured backend must fail loudly, not
#    silently create a second empty repo. One-off:
#      docker compose --profile backup run --rm --entrypoint rustic-mica backup init
if ! "$RUSTIC" cat config >/dev/null 2>&1; then
  log "repo not initialized (or backend unreachable). Initialize once with:"
  log "  docker compose --profile backup run --rm --entrypoint rustic-mica backup init"
  exit 1
fi

# 2) Export every workspace → mirrored Markdown+images tree (+ manifest.json).
#    `export` prunes files removed upstream, so the tree tracks current state.
log "export all workspaces → ${EXPORT_DIR}"
"$MICA_CLI" export --out "$EXPORT_DIR"

# 3) Snapshot each workspace as its own lineage: label = stable id, tag = name.
manifest="${EXPORT_DIR}/manifest.json"
count=0
while IFS=$'\t' read -r wsid wsname wsdir; do
  [ -n "$wsid" ] || continue
  path="${EXPORT_DIR}/${wsdir}"
  if [ ! -d "$path" ]; then
    log "skip ${wsid}: ${path} missing"
    continue
  fi
  log "snapshot ${wsdir} (ws=${wsname}) → label=${wsid}"
  "$RUSTIC" backup "$path" --label "$wsid" --tag "ws=${wsname}" --tag mica
  count=$((count + 1))
done < <(jq -r '.workspaces[] | [.id, .name, .dir] | @tsv' "$manifest")
log "snapshotted ${count} workspace(s)"

# 4) Retention PER workspace (group by the stable label = id), then prune once.
log "retention: keep ${KEEP_DAILY:-7}d / ${KEEP_WEEKLY:-4}w / ${KEEP_MONTHLY:-6}m per workspace + prune"
"$RUSTIC" forget \
  --group-by label \
  --keep-daily "${KEEP_DAILY:-7}" \
  --keep-weekly "${KEEP_WEEKLY:-4}" \
  --keep-monthly "${KEEP_MONTHLY:-6}" \
  --prune

log "done."
