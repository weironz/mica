#!/usr/bin/env bash
# Fast, offline regression checks for the production smoke gate.
set -euo pipefail
cd "$(dirname "$0")/.."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cat > "$tmp/curl" <<'CURL'
#!/usr/bin/env bash
url=${!#}
case "$url" in
  */api/ready) printf '{"version":"%s"}\n' "${FIXTURE_VERSION:-1.2.3}" ;;
  */main.dart.js) printf 'bundle bytes' ;;
  */mcp) echo 'verify-prod must not probe the SPA fallback /mcp' >&2; exit 99 ;;
  */)
    if [[ " $* " == *' -w '* ]]; then
      printf '%s' "${FIXTURE_INDEX_STATUS:-200}"
    else
      printf '%s' "${FIXTURE_INDEX_HTML:-<script src=\"flutter_bootstrap.js\"></script>}"
    fi
    ;;
  *) echo "unexpected URL: $url" >&2; exit 98 ;;
esac
CURL
chmod +x "$tmp/curl"
export PATH="$tmp:$PATH" SITE=https://example.test

expect_result() {
  local expected=$1 label=$2
  shift 2
  if env "$@" bash scripts/verify-prod.sh 1.2.3 >"$tmp/output" 2>&1; then
    result=pass
  else
    result=fail
  fi
  if [[ "$result" != "$expected" ]]; then
    echo "$label: expected $expected, got $result" >&2
    cat "$tmp/output" >&2
    exit 1
  fi
  echo "$label: $result"
}

expect_result pass healthy
expect_result fail homepage-500 FIXTURE_INDEX_STATUS=500
expect_result fail homepage-redirect FIXTURE_INDEX_STATUS=302
expect_result fail wrong-entry-page FIXTURE_INDEX_HTML='<html>upstream error</html>'
expect_result fail wrong-version FIXTURE_VERSION=9.9.9
