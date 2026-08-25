#!/usr/bin/env bash
set -euo pipefail
# Defaulted, same reason as restore-drill.sh: an injected-only variable under
# `set -u` makes a direct invocation fail on the variable rather than the task.
# release-check.sh already spelled it `${FLUTTER:-flutter}`; match it.
FLUTTER="${FLUTTER:-flutter}"
cd clients/mica_flutter
# Serial with a kill between files, exactly as CI does: the app is
# single-instance, and a lingering process makes the next launch fail with
# "Error waiting for a debug connection".
for f in integration_test/*.dart; do
  case "$(basename "$f")" in
    # Need the whole dev stack (`just dev`) — run those by hand.
    cloud_sync_test.dart|migration_sync_test.dart|offline_image_reconcile_test.dart|page_switch_fidelity_test.dart)
      echo "== skip $f (needs the dev stack; see the workflow header)"; continue;;
  esac
  echo "== $f"
  "${FLUTTER}" test "$f" -d windows
  powershell -NoProfile -Command "Get-Process mica_flutter -ErrorAction SilentlyContinue | Stop-Process -Force" || true
  sleep 3
done
