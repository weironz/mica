#!/usr/bin/env bash
# Check the fixed-name Web entry contract against a running deployment.
# Usage: bash scripts/web-entry-cache-check.sh https://example.test
set -euo pipefail
source "$(dirname "$0")/lib-local.sh"
site=${1:?usage: web-entry-cache-check.sh <origin>}
site=${site%/}
headers=$(mktemp)
trap 'rm -f "$headers"' EXIT

check_entry() {
  local path=$1 expected=$2 mime=${3:-} status cache content_type
  status=$(curl -sS --max-time 30 -D "$headers" -o /dev/null -w '%{http_code}' "$site$path")
  [ "$status" = "$expected" ] || fail "$path returned $status, expected $expected"
  if [ "$expected" = 200 ]; then
    cache=$(tr -d '\r' < "$headers" | grep -i '^cache-control:' || true)
    # no-store is the dev server's stronger no-cache policy.
    printf '%s\n' "$cache" | grep -Eiq '(^|[[:space:],])(no-cache|no-store)([[:space:],]|$)' \
      || fail "$path has no revalidation policy"
    if [ -n "$mime" ]; then
      content_type=$(tr -d '\r' < "$headers" | grep -i '^content-type:' || true)
      printf '%s\n' "$content_type" | grep -Eiq "^content-type:[[:space:]]*$mime([;[:space:]]|$)" \
        || fail "$path has an unexpected Content-Type (expected $mime)"
    fi
  fi
  note "$path: $status"
}

check_entry /index.html 200 text/html
for path in /flutter_bootstrap.js /main.dart.js; do
  check_entry "$path" 200 '(application|text)/(x-)?javascript'
done
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
note "Web entry contract: ALL GREEN"
