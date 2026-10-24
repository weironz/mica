#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
shopt -s nullglob
migrations=(migrations/*.sql)
[ ${#migrations[@]} -gt 0 ] || { echo 'REFUSED: no SQL migrations found' >&2; exit 1; }
failed=0
for migration in "${migrations[@]}"; do
  if ! git ls-files --error-unmatch -- "$migration" >/dev/null 2>&1; then
    echo "REFUSED: $migration is not tracked by Git; a clean deployment would omit it" >&2
    failed=1
  fi
done
[ "$failed" = 0 ]
echo "==> all ${#migrations[@]} SQL migrations are tracked"
