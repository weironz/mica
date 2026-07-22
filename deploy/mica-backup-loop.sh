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

# Dead man's switch. Ping ${HEALTHCHECK_URL} after a successful run and
# <URL>/fail after a failed one; a monitor (healthchecks.io etc.) alerts when
# the expected success ping stops arriving — which catches a wedged loop or a
# dead container that a stderr line reaches no one about. Best-effort: short
# timeouts, never fatal (kept out of `run`'s status), so a flaky monitor
# endpoint can't take the backup loop down with it. Unset URL → no-op.
#   $1 = "" for success, "/fail" for failure
ping_hc() {
  [ -n "${HEALTHCHECK_URL:-}" ] || return 0
  curl -fsS -m 10 --retry 3 -o /dev/null "${HEALTHCHECK_URL}${1:-}" \
    || echo "[$(date -Is)] mica-backup-loop: healthcheck ping failed (non-fatal)" >&2
}

run() {
  if /usr/local/bin/mica-backup.sh; then
    echo "[$(date -Is)] mica-backup-loop: run ok"
    ping_hc
  else
    echo "[$(date -Is)] mica-backup-loop: run FAILED (rc=$?)" >&2
    ping_hc /fail
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
