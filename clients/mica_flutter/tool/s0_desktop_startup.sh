#!/usr/bin/env bash
# S0 desktop startup baseline (external proxy).
#
# `main()` -> `runApp` has no hook in S0, and adding one would be a product-code
# change. This measures the closest external equivalent: process start -> the
# main window's title appears (engine up, first frame presented). It is NOT the
# same metric and is labelled as a proxy in the S0 execution record.
#
#   cd clients/mica_flutter
#   flutter build windows --release
#   bash tool/s0_desktop_startup.sh          # 5 runs, median
#   RUNS=1 bash tool/s0_desktop_startup.sh   # one run, for a sanity check
#
# Polling tasklist costs ~50-150 ms, so treat these numbers as coarse.
set -euo pipefail

repo_client=$(cd "$(dirname "$0")/.." && pwd)
exe="$repo_client/build/windows/x64/runner/Release/mica_flutter.exe"
runs=${RUNS:-5}

[ -f "$exe" ] || { echo "no release build at $exe — run: flutter build windows --release" >&2; exit 1; }
win_exe=$(cygpath -w "$exe" 2>/dev/null || printf '%s' "$exe")

taskkill //F //IM mica_flutter.exe >/dev/null 2>&1 || true

samples=()
for run in $(seq 1 "$runs"); do
  start=$(date +%s%3N)
  cmd //c start "" "$win_exe"
  found=""
  for _ in $(seq 1 1200); do
    line=$(tasklist //v //fi "IMAGENAME eq mica_flutter.exe" //fo csv //nh 2>/dev/null || true)
    if [ -n "$line" ]; then
      title=$(printf '%s\n' "$line" | head -1 | awk -F'","' '{print $NF}')
      case "$title" in
        '' | *N/A*) ;;
        *) found=$(( $(date +%s%3N) - start )); break ;;
      esac
    fi
    sleep 0.05
  done
  if [ -z "$found" ]; then
    echo "run $run: no window title within 60s" >&2
    taskkill //F //IM mica_flutter.exe >/dev/null 2>&1 || true
    exit 1
  fi
  samples+=("$found")
  printf '{"platform":"windows","item":"startup.window-shown","run":%d,"ms":%d}\n' "$run" "$found"
  taskkill //F //IM mica_flutter.exe >/dev/null 2>&1 || true
  sleep 1
done

sorted=$(printf '%s\n' "${samples[@]}" | sort -n)
n=${#samples[@]}
median=$(printf '%s\n' "$sorted" | sed -n "$(( (n + 1) / 2 ))p")
printf '{"platform":"windows","item":"startup.window-shown","median_ms":%s}\n' "$median"
