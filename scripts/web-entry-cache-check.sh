#!/usr/bin/env bash
# Check the fixed-name Web entry contract against a running deployment.
# Usage: bash scripts/web-entry-cache-check.sh https://example.test
set -euo pipefail
source "$(dirname "$0")/lib-local.sh"
site=${1:?usage: web-entry-cache-check.sh <origin>}
site=${site%/}
headers=$(mktemp)
trap 'rm -f "$headers"' EXIT

check_security_headers() {
  local path=$1 server
  for spec in \
    'strict-transport-security:max-age=[1-9][0-9]*' \
    'x-content-type-options:nosniff' \
    'x-frame-options:DENY' \
    'referrer-policy:strict-origin-when-cross-origin'; do
    local name=${spec%%:*} value=${spec#*:}
    tr -d '\r' < "$headers" | grep -Eiq "^$name:[[:space:]]*$value([;[:space:]]|$)" \
      || fail "$path is missing security header $name"
  done
  server=$(tr -d '\r' < "$headers" | grep -i '^server:' || true)
  [ -z "$server" ] || printf '%s\n' "$server" | grep -Eiq '^server:[[:space:]]*nginx[[:space:]]*$' \
    || fail "$path exposes an unexpected Server version"
  if [ "$path" = /index.html ]; then
    tr -d '\r' < "$headers" | grep -Eiq '^content-security-policy-report-only:.*wasm-unsafe-eval' \
      || fail "$path has no report-only application CSP"
    if tr -d '\r' < "$headers" | grep -Eiq '^content-security-policy:'; then
      fail "$path unexpectedly enforces CSP before the observation cycle"
    fi
  fi
}

check_entry() {
  local path=$1 expected=$2 mime=${3:-} revalidate=${4:-true} status cache content_type
  status=$(curl -sS --max-time 30 -D "$headers" -o /dev/null -w '%{http_code}' "$site$path")
  [ "$status" = "$expected" ] || fail "$path returned $status, expected $expected"
  if [ "$expected" = 200 ]; then
    cache=$(tr -d '\r' < "$headers" | grep -i '^cache-control:' || true)
    # no-store is the dev server's stronger no-cache policy.
    if [ "$revalidate" = true ]; then
      printf '%s\n' "$cache" | grep -Eiq '(^|[[:space:],])(no-cache|no-store)([[:space:],]|$)' \
        || fail "$path has no revalidation policy"
    fi
    if [ -n "$mime" ]; then
      content_type=$(tr -d '\r' < "$headers" | grep -i '^content-type:' || true)
      printf '%s\n' "$content_type" | grep -Eiq "^content-type:[[:space:]]*$mime([;[:space:]]|$)" \
        || fail "$path has an unexpected Content-Type (expected $mime)"
    fi
  fi
  check_security_headers "$path"
  note "$path: $status"
}

check_entry /index.html 200 text/html
for path in /flutter_bootstrap.js /main.dart.js; do
  check_entry "$path" 200 '(application|text)/(x-)?javascript'
done
# Deferred UI modules are also fixed-name deployment entrypoints.
check_entry /main.dart.js_1.part.js 200 '(application|text)/(x-)?javascript'
check_entry /main.dart.js_missing.part.js 404
for engine in mica_md mica_doc; do
  check_entry "/$engine/$engine.js" 200 '(application|text)/(x-)?javascript'
  check_entry "/$engine/${engine}_bg.wasm" 200 application/wasm
  check_entry "/$engine/missing.wasm" 404
done
# The editor's formula and Mermaid renderers load on demand from a worker.
check_entry /mica_preview_worker.js 200 '(application|text)/(x-)?javascript'
for kind in math mermaid; do
  check_entry "/mica_preview/$kind.js" 200 '(application|text)/(x-)?javascript'
  check_entry "/mica_preview/${kind}_bg.wasm" 200 application/wasm
done
check_entry /mica_preview/missing.wasm 404
check_entry /yjs_bundle.js 404
check_entry /mermaid.min.js 404
# Full CJK fonts are native-only. The Web still ships the code font and the
# engine's same-origin compressed shards; neither may fall through to HTML.
check_entry /assets/fonts/MaterialIcons-Regular.otf 200 font/otf false
check_entry /assets/fonts/RobotoMono-Regular.ttf 200 font/ttf false
check_entry /gfonts/roboto/v32/KFOmCnqEu92Fr1Me4GZLCzYlKw.woff2 200 font/woff2 false
check_entry /assets/fonts/NotoSansSC-Regular.otf 404
check_entry /assets/fonts/DroidSansFallback.ttf 404
check_entry /api/ready 200 application/json false
note "Web entry contract: ALL GREEN"
