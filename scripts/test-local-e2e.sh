#!/usr/bin/env bash
# Execute the real browser-install branch from local-e2e.sh with a stubbed npx.
# The full suites still prove browser launching; this test pins the shell's
# failure policy without downloading anything.
set -euo pipefail
cd "$(dirname "$0")/.."
block=$(python3 - <<'PY'
from pathlib import Path
text = Path('scripts/local-e2e.sh').read_text(encoding='utf-8')
start = text.index('  if [ -n "${CHROME_EXECUTABLE:-}" ]; then')
end = text.index('\n  fi\n', start) + len('\n  fi')
print(text[start:end], end='')
PY
)
[ -n "$block" ]
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT

run_branch() (
  set -e
  fail() { printf 'REFUSED: %s\n' "$*" >&2; exit 19; }
  npx() {
    printf '%s\n' "$*" >> "$CALLS"
    return "${NPX_RC:-0}"
  }
  eval "$block"
)

# A failed bundled download is only an installation fallback. The suites must
# continue to browser.mjs, which tries system Chrome/Edge and still enforces
# every assertion.
unset CHROME_EXECUTABLE
CALLS="$fixture/fallback.calls" NPX_RC=23 run_branch > "$fixture/fallback.out"
grep -q 'system Chrome/Edge' "$fixture/fallback.out"
grep -qx 'playwright install chromium' "$fixture/fallback.calls"
echo 'PASS: bundled Chromium download failure falls through to system Chrome/Edge'

# A successful install takes the bundled path exactly once.
CALLS="$fixture/bundled.calls" NPX_RC=0 run_branch > /dev/null
grep -qx 'playwright install chromium' "$fixture/bundled.calls"
echo 'PASS: bundled Chromium installation remains the default'

# An explicit executable bypasses installation entirely.
# Git Bash's -x test recognizes Windows executable suffixes.
touch "$fixture/chrome.exe"
chmod +x "$fixture/chrome.exe"
CHROME_EXECUTABLE="$fixture/chrome.exe" CALLS="$fixture/explicit.calls" NPX_RC=0 run_branch
[ ! -e "$fixture/explicit.calls" ]
echo 'PASS: explicit Chrome bypasses Playwright installation'

# A bad explicit path is a configuration error, not a reason to hide it by
# choosing another browser.
if CHROME_EXECUTABLE="$fixture/missing" CALLS="$fixture/invalid.calls" NPX_RC=0 \
    run_branch > "$fixture/invalid.out" 2>&1; then
  echo 'FAIL: invalid CHROME_EXECUTABLE was accepted' >&2
  exit 1
fi
grep -q 'CHROME_EXECUTABLE is not executable' "$fixture/invalid.out"
[ ! -e "$fixture/invalid.calls" ]
echo 'PASS: invalid explicit Chrome is refused without a fallback'
