#!/usr/bin/env bash
# Daily scheduler for the containerised mica-cli backup. Runs mica-backup.sh
# (export → snapshot → forget --prune) once per day at ${BACKUP_HOUR}:00
# (container-local time), and once immediately on (re)start unless disabled.
#
# Env:
#   BACKUP_HOUR      hour of day to run, 0-23 (default 3)
#   BACKUP_ON_START  run once right after the container starts (default 1)
# Plus everything mica-backup.sh reads (MICA_SERVER/MICA_TOKEN/MICA_BACKUP_*).
#
# NB: no `set -e` — a failed run must be logged, not kill the loop.
set -uo pipefail

HOUR="${BACKUP_HOUR:-3}"

run() {
  if /usr/local/bin/mica-backup.sh; then
    echo "[$(date -Is)] mica-backup-loop: run ok"
  else
    echo "[$(date -Is)] mica-backup-loop: run FAILED (rc=$?)" >&2
  fi
}

[ "${BACKUP_ON_START:-1}" = "1" ] && run

while true; do
  now="$(date +%s)"
  next="$(date -d "today ${HOUR}:00" +%s)"
  [ "$next" -le "$now" ] && next="$(date -d "tomorrow ${HOUR}:00" +%s)"
  wait_s=$(( next - now ))
  echo "[$(date -Is)] mica-backup-loop: sleeping ${wait_s}s until ${HOUR}:00"
  sleep "$wait_s"
  run
done
