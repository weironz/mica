#!/usr/bin/env bash
# S0 external desktop proxy: process start -> window title (not main -> runApp).
# From clients/mica_flutter: bash tool/s0_desktop_startup.sh
# Five runs by default; RUNS=1 for a sanity check. Polling costs ~50-150 ms.
set -euo pipefail

repo_client=$(cd "$(dirname "$0")/.." && pwd)
exe="$repo_client/build/windows/x64/runner/Release/mica_flutter.exe"
runs=${RUNS:-5}

# Never close a user's window, even when the single-instance mutex blocks us.
existing=$(tasklist //fi "IMAGENAME eq mica_flutter.exe" //fo csv //nh)
if [[ "$existing" == *'"mica_flutter.exe"'* ]]; then
  printf '%s\n' 'Mica is already running; close it yourself before measuring.' >&2
  exit 1
fi
[ -f "$exe" ] || { printf 'no release build at %s\n' "$exe" >&2; exit 1; }

app_pid=''
cleanup() {
  if [ -n "$app_pid" ]; then
    # $! is our child, but taskkill needs the WinPID rather than the MSYS PID.
    # Resolve it while the child is alive; never kill by image name.
    win_pid=$(ps -p "$app_pid" | awk 'NR == 2 {print $4}' || true)
    if [[ "$win_pid" =~ ^[0-9]+$ ]]; then
      taskkill //F //PID "$win_pid" >/dev/null 2>&1 || true
    fi
    wait "$app_pid" 2>/dev/null || true
    app_pid=''
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

samples=()
for run in $(seq 1 "$runs"); do
  start=$(date +%s%3N)
  "$exe" &
  app_pid=$!
  found=''
  deadline=$((SECONDS + 60))
  while [ "$SECONDS" -lt "$deadline" ]; do
    kill -0 "$app_pid" 2>/dev/null || { printf 'run %s: app exited\n' "$run" >&2; exit 1; }
    win_pid=$(ps -p "$app_pid" | awk 'NR == 2 {print $4}' || true)
    if [[ "$win_pid" =~ ^[0-9]+$ ]]; then
      line=$(tasklist //v //fi "PID eq $win_pid" //fo csv //nh)
      if [[ "$line" == *'"mica_flutter.exe"'* ]]; then
        title=$(printf '%s\n' "$line" | head -1 | awk -F'","' '{print $NF}')
        case "$title" in
          '' | *N/A*) ;;
          *) found=$(( $(date +%s%3N) - start )); break ;;
        esac
      fi
    fi
    sleep 0.05
  done
  [ -n "$found" ] || { printf 'run %s: no window title within 60s\n' "$run" >&2; exit 1; }
  samples+=("$found")
  printf '{"platform":"windows","item":"startup.window-shown","run":%d,"ms":%d}\n' "$run" "$found"
  cleanup
  sleep 1
done

median=$(printf '%s\n' "${samples[@]}" | sort -n | sed -n "$(( (${#samples[@]} + 1) / 2 ))p")
printf '{"platform":"windows","item":"startup.window-shown","median_ms":%s}\n' "$median"
