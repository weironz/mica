#!/usr/bin/env bash
set -euo pipefail
echo 'ready=false' >> "$GITHUB_OUTPUT"
command -v jq >/dev/null 2>&1 || exit 0
want=$(jq -er .flutter "$GITHUB_WORKSPACE/.fvmrc") || exit 0
root="${FLUTTER_ROOT:-${RUNNER_TOOL_CACHE%/}/flutter/$want/stable}"
stamp="$root/bin/cache/flutter.version.json"
[ -f "$stamp" ] && [ -f "$root/bin/flutter" ] || exit 0

# Read the SDK's own stamp without warming the Flutter CLI.
actual=$(jq -er .frameworkVersion "$stamp" 2>/dev/null) || exit 0
channel=$(jq -er .channel "$stamp" 2>/dev/null) || exit 0
[ "$actual" = "$want" ] && [ "$channel" = stable ] || exit 0
if [ -d "$root/.git" ]; then
  revision=$(git -C "$root" rev-parse HEAD 2>/dev/null) || exit 0
  stamped_revision=$(jq -er .frameworkRevision "$stamp" 2>/dev/null) || exit 0
  [ "$revision" = "$stamped_revision" ] || exit 0
fi

echo "$root/bin" >> "$GITHUB_PATH"
echo "FLUTTER_ROOT=$root" >> "$GITHUB_ENV"
echo 'ready=true' >> "$GITHUB_OUTPUT"
echo "Using pre-installed Flutter $want (stable) at $root"
