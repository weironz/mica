#!/usr/bin/env bash
# Quick-launch Mica for manual verification (from the current working tree).
#
#   ./run.sh          # desktop — local mode, no backend needed
#   ./run.sh web      # web in Chrome (cloud login needed for data)
#
# Hot reload: after a code change, press  r  in this terminal to see it live
# (no rebuild); R = full restart, q = quit. Desktop opens the offline "local"
# world, so you can create folders/pages and try things with no server/login.
set -euo pipefail

target="${1:-windows}"
case "$target" in
  web | chrome) device=chrome ;;
  windows | desktop) device=windows ;;
  *) device="$target" ;;
esac

echo "Launching Mica on '$device'  (hot reload: r | restart: R | quit: q)"
cd "$(dirname "$0")/clients/mica_flutter"
exec flutter run -d "$device"
