#!/usr/bin/env bash
set -euo pipefail
cd e2e
# `npm ci` wants a lockfile; install is fine for one pinned dependency.
[ -d node_modules ] || npm install --no-audit --no-fund
# From CN this download is blocked; the script falls back to system Chrome, so
# a failure here is not fatal.
npx playwright install chromium \
  || echo "note: bundled Chromium unavailable — the script will use system Chrome"
node web_e2e.mjs --base http://127.0.0.1:8090 \
                 --email "$1" --password "$2"
