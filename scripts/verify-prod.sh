#!/usr/bin/env bash
# Prove prod really serves <version> (pure-Rust API + pure-Rust WebAssembly bundle).
set -euo pipefail
SITE="${SITE:-https://mica.cloudcele.com}"

version=$1
got=$(curl -fsS "$SITE"/api/ready)
echo "$got"
echo "$got" | grep -q "\"version\":\"$version\"" \
  || { echo "VERSION MISMATCH: $SITE is not serving $version"; exit 1; }
index_status=$(curl -sS -o /dev/null -w '%{http_code}' "$SITE"/)
echo "index:  $index_status"
[[ "$index_status" == 200 ]] || { echo "INDEX FAILED: $SITE/ returned $index_status"; exit 1; }
index_html=$(curl -fsS "$SITE"/)
[[ "$index_html" == *mica_rust_bg.wasm* ]] \
  || { echo "INDEX FAILED: $SITE/ did not serve the pure-Rust Web entry page"; exit 1; }
echo "bundle: $(curl -fsS "$SITE"/mica_rust_bg.wasm | md5sum | cut -c1-12)…"
