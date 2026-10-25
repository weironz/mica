#!/bin/sh
# SQLx stores byte-level checksums. Git's text filters can hide CRLF drift.
set -eu
directory=${1:-migrations}
found=false
failed=false
for migration in "$directory"/*.sql; do
  [ -f "$migration" ] || continue
  found=true
  # Windows text-mode grep may remove CR before matching; inspect raw bytes.
  bytes=$(LC_ALL=C od -An -t x1 "$migration") || {
    echo "REFUSED: could not read raw migration bytes: $migration" >&2
    exit 1
  }
  case "$bytes" in
    *0d*)
      echo "REFUSED: $migration contains CRLF/CR bytes; restore its committed LF bytes before building" >&2
      failed=true
      ;;
  esac
done
[ "$found" = true ] || { echo "REFUSED: no SQL migrations found in $directory" >&2; exit 1; }
[ "$failed" = false ] || exit 1
echo "==> SQL migration bytes: LF only"
