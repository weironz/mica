#!/usr/bin/env bash
# Editor/UI and deferred-module browser regressions; no API or database required.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
require_flutter_pin "browser regressions test the pin"
bash scripts/build-md-wasm.sh
bash scripts/build-doc-wasm.sh
bash scripts/build-preview-wasm.sh
(
  cd e2e
  npm ci --no-audit --no-fund
  if [ -n "${CHROME_EXECUTABLE:-}" ]; then
    [ -x "$CHROME_EXECUTABLE" ] || fail "CHROME_EXECUTABLE is not executable"
  else
    npx playwright install chromium
  fi
)
(
  cd clients/mica_flutter
  "$FLUTTER" build web --release --no-web-resources-cdn \
    --target=e2e/web_harness_entry.dart \
    --dart-define=MICA_DEV_AUTOLOGIN=false \
    --dart-define=MICA_API_BASE_URL=http://127.0.0.1:8093 \
    --dart-define=MICA_CLOUD_URL=http://127.0.0.1:8093 \
    --output="$PWD/build/web_harness"
)

log=$(mktemp)
server=
cleanup() {
  if [ -n "$server" ]; then
    kill "$server" 2>/dev/null || true
    wait "$server" 2>/dev/null || true
  fi
  rm -f "$log"
}
trap cleanup EXIT
python3 -m http.server 8091 --bind 127.0.0.1 \
  --directory clients/mica_flutter/build/web_harness > "$log" 2>&1 &
server=$!
for _ in $(seq 1 20); do
  kill -0 "$server" 2>/dev/null || { cat "$log"; fail "harness server exited"; }
  if curl -fsS http://127.0.0.1:8091/ > /dev/null; then break; fi
  sleep 1
done
kill -0 "$server" 2>/dev/null || { cat "$log"; fail "harness server exited"; }
curl -fsS http://127.0.0.1:8091/ > /dev/null
export MICA_UI_RESULTS_DIR=${MICA_UI_RESULTS_DIR:-"$PWD/clients/mica_flutter/build/ui-experience-results"}
(
  cd e2e
  node cdp_eval_test.mjs
  node rich_preview_e2e.mjs --base 'http://127.0.0.1:8091/?harness=clipboard'
  node clipboard_e2e.mjs --base 'http://127.0.0.1:8091/?harness=clipboard'
)
node e2e/ui_experience_e2e.mjs
node e2e/sidebar_rename_e2e.mjs

node e2e/deferred_modules_e2e.mjs
