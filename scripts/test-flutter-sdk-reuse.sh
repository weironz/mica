#!/usr/bin/env bash
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
helper="$repo/.github/actions/setup-flutter/reuse.sh"
tmp_root=$(cd "${TMPDIR:-/tmp}" && pwd -P)
fixture=$(mktemp -d "$tmp_root/mica-sdk-reuse.XXXXXX")
fixture=$(cd "$fixture" && pwd -P)
case "$fixture" in "$tmp_root"/mica-sdk-reuse.*) ;; *) exit 1 ;; esac
trap 'rm -rf -- "$fixture"' EXIT

command -v jq >/dev/null
export GITHUB_WORKSPACE="$fixture" RUNNER_TOOL_CACHE="$fixture/tools"
export GITHUB_OUTPUT="$fixture/output" GITHUB_ENV="$fixture/env" GITHUB_PATH="$fixture/path"
unset FLUTTER_ROOT
printf '{"flutter":"3.47.1"}\n' > "$fixture/.fvmrc"
sdk="$RUNNER_TOOL_CACHE/flutter/3.47.1/stable"

check() {
  local expected=$1
  : > "$GITHUB_OUTPUT"; : > "$GITHUB_ENV"; : > "$GITHUB_PATH"
  bash "$helper" >/dev/null
  [ "$(tail -n 1 "$GITHUB_OUTPUT")" = "ready=$expected" ]
  if [ "$expected" = false ]; then
    [ ! -s "$GITHUB_ENV" ] && [ ! -s "$GITHUB_PATH" ]
  else
    grep -Fxq "${FLUTTER_ROOT:-$sdk}/bin" "$GITHUB_PATH"
  fi
}

check false
mkdir -p "$sdk/bin/cache"
printf '#!/usr/bin/env bash\nexit 99\n' > "$sdk/bin/flutter"
printf '{"frameworkVersion":"3.47.1","channel":"stable"}\n' > "$sdk/bin/cache/flutter.version.json"
# A valid stamp can be reused without executing the Flutter stub.
check true
export FLUTTER_ROOT="$sdk"
check true
printf '{"frameworkVersion":"0.0.0","channel":"stable"}\n' > "$sdk/bin/cache/flutter.version.json"
check false
printf '{"frameworkVersion":"3.47.1","channel":"beta"}\n' > "$sdk/bin/cache/flutter.version.json"
check false
printf 'invalid json\n' > "$sdk/bin/cache/flutter.version.json"
check false
echo 'Flutter SDK reuse: valid, absent, stale, wrong-channel and malformed SDKs passed'
