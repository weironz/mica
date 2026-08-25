#!/usr/bin/env bash
set -euo pipefail
got=$(curl -fsS "${SITE}"/api/ready)
echo "$got"
echo "$got" | grep -q "\"version\":\"$1\"" \
  || { echo "VERSION MISMATCH: prod is not "$1""; exit 1; }
echo "mcp:   $(curl -s -o /dev/null -w '%{http_code}' "${SITE}"/mcp)"
echo "index: $(curl -s -o /dev/null -w '%{http_code}' "${SITE}"/)"
echo "bundle: $(curl -fsS "${SITE}"/main.dart.js | md5sum | cut -c1-12)…"
