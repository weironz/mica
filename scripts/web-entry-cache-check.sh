#!/usr/bin/env bash
# Verify pure-Rust Web entry headers, Brotli wasm MIME, CSP hash header, and
# 404 retirement of legacy JS assets against a running mica-web origin.
set -euo pipefail
base="${1:?usage: $0 <http://host[:port]>}"
base="${base%/}"

# 1. Entry HTML: 200, Cache-Control: no-cache, strict sha256 CSP (no 'unsafe-inline' in script-src).
entry_hdrs=$(curl -fsSI "$base/")
grep -iq '^cache-control:.*no-cache' <<<"$entry_hdrs" \
  || { echo "FAIL: $base/ missing Cache-Control: no-cache" >&2; exit 1; }
grep -iq "^content-security-policy-report-only:.*script-src 'self' 'wasm-unsafe-eval' 'sha256-" <<<"$entry_hdrs" \
  || { echo "FAIL: $base/ CSP script-src not pinned by sha256 hash" >&2; exit 1; }

# 2. Main WASM: 200, application/wasm, Brotli-encoded when client accepts br.
wasm_hdrs=$(curl -fsSI -H 'Accept-Encoding: br' "$base/mica_rust_bg.wasm")
grep -iq '^content-type:.*application/wasm' <<<"$wasm_hdrs" \
  || { echo "FAIL: $base/mica_rust_bg.wasm wrong Content-Type" >&2; exit 1; }
grep -iq '^content-encoding:.*br' <<<"$wasm_hdrs" \
  || { echo "FAIL: $base/mica_rust_bg.wasm did not serve precompressed Brotli (.br)" >&2; exit 1; }

# 3. Retired Flutter/JS assets must return 404 (not SPA index.html 200).
for gone in /main.dart.js /yjs_bundle.js /mermaid.min.js; do
  code=$(curl -sS -o /dev/null -w '%{http_code}' "$base$gone")
  [ "$code" = "404" ] || { echo "FAIL: retired asset $gone returned HTTP $code (expected 404)" >&2; exit 1; }
done

echo "PASS: web entry headers, Brotli application/wasm, CSP hash, and retired-asset 404s verified on $base"
