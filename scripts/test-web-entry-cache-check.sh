#!/usr/bin/env bash
# Exercise the deployed-entry checks without contacting a server.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
cat > "$fixture/curl" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
headers=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -D) headers=$2; shift 2 ;;
    --max-time|-o|-w) shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
path=${url#https://example.test}
[ "$path" != "${FAIL_NETWORK:-}" ] || exit 7
status=200
case "$path" in */missing.wasm|/yjs_bundle.js) status=404 ;; esac
[ "$path" != "${BAD_STATUS:-}" ] || status=200
[ "$path" != "${NOT_FOUND:-}" ] || status=404
cache=${VALID_CACHE:-no-cache}
[ "$path" != "${BAD_CACHE:-}" ] || cache=public
mime=application/javascript
case "$path" in *.wasm) mime=application/wasm ;; *.html) mime=text/html ;; esac
[ "$path" != "${BAD_MIME:-}" ] || mime=text/html
printf 'HTTP/1.1 %s\r\nCache-Control: %s\r\nContent-Type: %s\r\n\r\n' "$status" "$cache" "$mime" > "$headers"
printf '%s' "$status"
STUB
chmod +x "$fixture/curl"
check() {
  local expected=$1 label=$2 status=0
  shift 2
  env PATH="$fixture:$PATH" "$@" bash scripts/web-entry-cache-check.sh https://example.test > "$fixture/output" 2>&1 || status=$?
  if [ "$expected" = pass ]; then
    if [ "$status" != 0 ] || ! grep -q 'Web entry contract: ALL GREEN' "$fixture/output"; then
      printf 'FAIL: %s did not reach ALL GREEN\n' "$label" >&2
      exit 1
    fi
  elif [ "$status" = 0 ]; then
    printf 'FAIL: %s passed unexpectedly\n' "$label" >&2
    exit 1
  fi
  printf 'PASS: %s\n' "$label"
}
check pass 'current entries'
check pass 'development no-store policy' VALID_CACHE=no-store
check fail 'missing document glue' NOT_FOUND=/mica_doc/mica_doc.js
check fail 'cached document wasm' BAD_CACHE=/mica_doc/mica_doc_bg.wasm
check fail 'cached Markdown glue' BAD_CACHE=/mica_md/mica_md.js
check fail 'cached Flutter entry' BAD_CACHE=/main.dart.js
check fail 'HTML in place of wasm' BAD_MIME=/mica_doc/mica_doc_bg.wasm
check fail 'HTML in place of Flutter bootstrap' BAD_MIME=/flutter_bootstrap.js
check fail 'HTML in place of Flutter JS' BAD_MIME=/main.dart.js
check fail 'HTML in place of engine glue' BAD_MIME=/mica_doc/mica_doc.js
check fail 'retired yjs entry falls through to SPA' BAD_STATUS=/yjs_bundle.js
check fail 'missing engine falls through to SPA' BAD_STATUS=/mica_md/missing.wasm
check fail 'network failure' FAIL_NETWORK=/index.html
printf 'Web entry gate regressions: ALL GREEN\n'
