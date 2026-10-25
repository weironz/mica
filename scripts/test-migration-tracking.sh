#!/usr/bin/env bash
set -euo pipefail
source_root=$(cd "$(dirname "$0")/.." && pwd)
temp_parent=$(cd "${TMPDIR:-/tmp}" && pwd)
fixture=$(mktemp -d "$temp_parent/mica-migration-tracking.XXXXXX")
[[ "$fixture" == "$temp_parent"/mica-migration-tracking.* ]] || exit 2
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/migrations"
cp "$source_root/scripts/check-migration-tracking.sh" "$fixture/scripts/"
cp "$source_root/scripts/check-migration-bytes.sh" "$fixture/scripts/"
git -C "$fixture" init -q
git -C "$fixture" config core.autocrlf true
printf 'migrations/*.sql text eol=lf\n' > "$fixture/.gitattributes"
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
printf 'SELECT 1;\r\n' > "$fixture/migrations/0001_initial.sql"
git -C "$fixture" diff --quiet -- migrations/0001_initial.sql || {
  echo 'FAIL: fixture must reproduce clean Git status with CRLF worktree bytes' >&2
  exit 1
}
if bash "$fixture/scripts/check-migration-tracking.sh" > "$fixture/bytes-refusal.log" 2>&1; then
  echo 'FAIL: Git-normalized CRLF migration was accepted' >&2
  exit 1
fi
grep -qF 'contains CRLF/CR bytes' "$fixture/bytes-refusal.log"
mkdir -p "$fixture/bin"
printf '#!/bin/sh\nexit 2\n' > "$fixture/bin/od"
chmod +x "$fixture/bin/od"
if PATH="$fixture/bin:$PATH" sh "$source_root/scripts/check-migration-bytes.sh" "$fixture/migrations" > "$fixture/reader-refusal.log" 2>&1; then
  echo 'FAIL: failed byte reader was accepted' >&2
  exit 1
fi
grep -qF 'could not read raw migration bytes' "$fixture/reader-refusal.log"
echo 'PASS: tracked/ignored SQL, Git-clean CRLF, and failed byte-reader regressions'
