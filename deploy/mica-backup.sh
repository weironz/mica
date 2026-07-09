#!/usr/bin/env bash
# One scheduled Mica backup run: export all workspaces (Markdown + images) →
# snapshot into the restic-format repo (local + Aliyun OSS) → apply retention.
#
# All config comes from the environment (the systemd unit's EnvironmentFile,
# root-600) so no secret ever lands on the command line / in `ps`:
#
#   MICA_SERVER            e.g. https://mica.cloudcele.com   (for `mica-cli export`)
#   MICA_TOKEN             a Mica access token
#   MICA_BACKUP_REPO       e.g. opendal:s3:/mica
#   MICA_BACKUP_PASSWORD   the repo encryption password
#   MICA_BACKUP_OPTS       backend options incl. OSS creds, e.g.
#     "bucket=… endpoint=https://oss-cn-hangzhou.aliyuncs.com region=oss-cn-hangzhou \
#      access_key_id=… secret_access_key=… enable_virtual_host_style=true"
#
# Optional: MICA_EXPORT_DIR (default /var/lib/mica/export),
#           KEEP_DAILY/KEEP_WEEKLY/KEEP_MONTHLY (default 14/8/6),
#           MICA_CLI / MICA_BACKUP (binary paths).
set -euo pipefail

MICA_CLI="${MICA_CLI:-mica-cli}"
MICA_BACKUP="${MICA_BACKUP:-mica-backup}"
EXPORT_DIR="${MICA_EXPORT_DIR:-/var/lib/mica/export}"

log() { echo "[$(date -Is)] mica-backup: $*"; }

log "export all workspaces → ${EXPORT_DIR}"
"$MICA_CLI" export --out "$EXPORT_DIR"

log "snapshot ${EXPORT_DIR} → ${MICA_BACKUP_REPO}"
"$MICA_BACKUP" snapshot --path "$EXPORT_DIR" --tag mica

log "retention: keep ${KEEP_DAILY:-14}d / ${KEEP_WEEKLY:-8}w / ${KEEP_MONTHLY:-6}m + prune"
"$MICA_BACKUP" forget \
  --keep-daily "${KEEP_DAILY:-14}" \
  --keep-weekly "${KEEP_WEEKLY:-8}" \
  --keep-monthly "${KEEP_MONTHLY:-6}" \
  --prune

log "done."
