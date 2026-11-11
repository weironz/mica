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
case "$path" in
  */missing.wasm|/main.dart.js_missing.part.js|/yjs_bundle.js|/mermaid.min.js|/assets/fonts/NotoSansSC-Regular.otf|/assets/fonts/DroidSansFallback.ttf) status=404 ;;
esac
[ "$path" != "${BAD_STATUS:-}" ] || status=200
[ "$path" != "${NOT_FOUND:-}" ] || status=404
cache=${VALID_CACHE:-no-cache}
[ "$path" != "${BAD_CACHE:-}" ] || cache=public
mime=application/javascript
case "$path" in
  *.wasm) mime=application/wasm ;;
  *.html) mime=text/html ;;
  *.ttf) mime=font/ttf ;;
  *.otf) mime=font/otf ;;
  *.woff2) mime=font/woff2 ;;
  /api/ready) mime=application/json ;;
esac
[ "$path" != "${BAD_MIME:-}" ] || mime=text/html
printf 'HTTP/1.1 %s\r\nCache-Control: %s\r\nContent-Type: %s\r\n' "$status" "$cache" "$mime" > "$headers"
for spec in 'Strict-Transport-Security:max-age=31536000' 'X-Content-Type-Options:nosniff' 'X-Frame-Options:DENY' 'Referrer-Policy:strict-origin-when-cross-origin'; do
  name=${spec%%:*}; value=${spec#*:}
  [ "$name" != "${MISSING_HEADER:-}" ] || [ "$path" != "${HEADER_PATH:-/index.html}" ] || continue
  printf '%s: %s\r\n' "$name" "$value" >> "$headers"
done
printf 'Server: %s\r\n' "${SERVER_VALUE:-nginx}" >> "$headers"
if [ "$path" = /index.html ] && [ -z "${MISSING_CSP:-}" ]; then
  printf "Content-Security-Policy-Report-Only: default-src 'self'; script-src 'self' 'wasm-unsafe-eval'\r\n" >> "$headers"
fi
[ -z "${ENFORCED_CSP:-}" ] || printf "Content-Security-Policy: default-src 'self'\r\n" >> "$headers"
printf '\r\n' >> "$headers"
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
check fail 'missing formula renderer' NOT_FOUND=/mica_preview/math_bg.wasm
check fail 'missing preview worker' NOT_FOUND=/mica_preview_worker.js
check fail 'cached Mermaid renderer' BAD_CACHE=/mica_preview/mermaid_bg.wasm
check fail 'HTML in place of renderer wasm' BAD_MIME=/mica_preview/math_bg.wasm
check fail 'missing renderer falls through to SPA' BAD_STATUS=/mica_preview/missing.wasm
check fail 'retired Mermaid script falls through to SPA' BAD_STATUS=/mermaid.min.js
check fail 'wrong code font MIME' BAD_MIME=/assets/fonts/RobotoMono-Regular.ttf
check fail 'missing code font' NOT_FOUND=/assets/fonts/RobotoMono-Regular.ttf
check fail 'wrong fallback shard MIME' BAD_MIME=/gfonts/roboto/v32/KFOmCnqEu92Fr1Me4GZLCzYlKw.woff2
check fail 'missing fallback shard' NOT_FOUND=/gfonts/roboto/v32/KFOmCnqEu92Fr1Me4GZLCzYlKw.woff2
check fail 'wrong icon font MIME' BAD_MIME=/assets/fonts/MaterialIcons-Regular.otf
check fail 'full CJK font is still bundled' BAD_STATUS=/assets/fonts/NotoSansSC-Regular.otf
for header in Strict-Transport-Security X-Content-Type-Options X-Frame-Options Referrer-Policy; do
  check fail "missing $header" MISSING_HEADER="$header"
done
check fail 'headers lost in API location' MISSING_HEADER=X-Frame-Options HEADER_PATH=/api/ready
check fail 'headers lost on an error response' MISSING_HEADER=X-Content-Type-Options HEADER_PATH=/mica_md/missing.wasm
check fail 'nginx version exposed' SERVER_VALUE=nginx/1.31.2
check fail 'missing report-only CSP' MISSING_CSP=1
check fail 'CSP enforced before observation' ENFORCED_CSP=1

# The report-only policy must allow the actual fixed bootstrap scripts after
# future index.html edits; a policy that only looks plausible is insufficient.
python3 - <<'PY_CSP'
import base64
import hashlib
import pathlib
import re
root = pathlib.Path('.')
html = (root / 'clients/mica_flutter/web/index.html').read_text()
for attrs, body in re.findall(r'<script([^>]*)>([\s\S]*?)</script>', html):
    if 'src=' in attrs:
        continue
    digest = base64.b64encode(hashlib.sha256(body.encode()).digest()).decode()
    for path in ['deploy/nginx.conf', 'deploy/nginx.dev.conf']:
        if f"'sha256-{digest}'" not in (root / path).read_text():
            raise SystemExit(f'{path}: CSP hash missing for actual bootstrap script')
print('PASS: both observation policies permit the actual inline bootstraps')
PY_CSP
check fail 'cached deferred module' BAD_CACHE=/main.dart.js_1.part.js
check fail 'missing deferred module' NOT_FOUND=/main.dart.js_1.part.js
check fail 'HTML in place of deferred module' BAD_MIME=/main.dart.js_1.part.js
check fail 'missing deferred module falls through to SPA' BAD_STATUS=/main.dart.js_missing.part.js
