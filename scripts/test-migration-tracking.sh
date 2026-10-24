#!/usr/bin/env bash
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
temp_parent=$(cd "${TMPDIR:-/tmp}" && pwd)
fixture=$(mktemp -d "$temp_parent/mica-migration-tracking.XXXXXX")
[[ "$fixture" == "$temp_parent"/mica-migration-tracking.* ]] || exit 2
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/migrations"
cp "$source_root/scripts/check-migration-tracking.sh" "$fixture/scripts/"
git -C "$fixture" init -q
printf '*credential*\n' > "$fixture/.gitignore"
printf 'SELECT 1;\n' > "$fixture/migrations/0001_initial.sql"
git -C "$fixture" add migrations/0001_initial.sql
bash "$fixture/scripts/check-migration-tracking.sh"
printf 'SELECT 2;\n' > "$fixture/migrations/0034_user_credential_epoch.sql"
if bash "$fixture/scripts/check-migration-tracking.sh" > "$fixture/refusal.log" 2>&1; then
  echo 'FAIL: an ignored migration was accepted' >&2
  exit 1
fi
rg_pattern='0034_user_credential_epoch.sql is not tracked'
grep -qF "$rg_pattern" "$fixture/refusal.log"
git -C "$fixture" add -f migrations/0034_user_credential_epoch.sql
bash "$fixture/scripts/check-migration-tracking.sh"
echo 'PASS: tracked SQL accepted; ignored SQL refused; explicitly tracked SQL accepted'
