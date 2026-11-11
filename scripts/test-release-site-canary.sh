#!/usr/bin/env bash
# A changed working tree must not substitute its contract for the target tag's.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/bin" "$fixture/repo/scripts"
cp "$repo/scripts/release-site-canary.sh" "$repo/scripts/lib-local.sh" "$fixture/repo/scripts/"
printf '#!/usr/bin/env bash\nexit 0\n' > "$fixture/repo/scripts/web-entry-cache-check.sh"
cat > "$fixture/bin/git" <<'STUB'
#!/usr/bin/env bash
case "$1" in
  rev-parse) exit 0 ;;
  cat-file) [ "${OLD_CONTRACT:-0}" != 1 ] ;;
  show)
    case "$2" in
      *lib-local.sh) cat "$SITE_FIXTURE/repo/scripts/lib-local.sh" ;;
      *web-entry-cache-check.sh) printf '#!/usr/bin/env bash\n[ "${TAG_HEADER_OK:-yes}" = yes ] || exit 7\necho "target tag contract ran"\n' ;;
      *) exit 2 ;;
    esac ;;
  *) exit 3 ;;
esac
STUB
cat > "$fixture/bin/curl" <<'STUB'
#!/usr/bin/env bash
printf '{"version":"%s"}\n' "${SITE_VERSION:-1.2.3}"
STUB
chmod +x "$fixture/bin/"*
export SITE_FIXTURE="$fixture" PATH="$fixture/bin:$PATH"
check() {
  local expected=$1 label=$2 rc=0
  shift 2
  env "$@" bash "$fixture/repo/scripts/release-site-canary.sh" 1.2.3 https://example.test > "$fixture/output" 2>&1 || rc=$?
  if [ "$expected" = pass ]; then
    [ "$rc" -eq 0 ] && grep -q 'target tag contract ran' "$fixture/output" || { cat "$fixture/output"; exit 1; }
  else
    [ "$rc" -ne 0 ] || { echo "FAIL: $label passed"; exit 1; }
  fi
  echo "PASS: $label"
}
check pass 'target-tag contract runs'
check fail 'health version mismatch' SITE_VERSION=1.2.30
check fail 'target-tag header failure survives a permissive working tree' TAG_HEADER_OK=no
check fail 'old releases do not pretend to support the new contract' OLD_CONTRACT=1
