#!/usr/bin/env bash
# AFTER deployment: execute the fixed-entry contract belonging to the target tag.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
version=${1:?usage: release-site-canary.sh X.Y.Z https://origin}
site=${2:?usage: release-site-canary.sh X.Y.Z https://origin}
check_version_arg "$version"
[[ "$site" =~ ^https?://[^/]+/?$ ]] || fail "site must be an http(s) origin"
site=${site%/}
tag="v$version"
if ! git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  git fetch origin "refs/tags/$tag:refs/tags/$tag"
fi
# Old 0.14.4 predates this contract. Do not silently test it with weaker rules.
git cat-file -e "$tag:scripts/release-canary.sh" 2>/dev/null \
  || fail "$tag predates the release canary contract; this check does not apply to it"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
for script in web-entry-cache-check.sh lib-local.sh; do
  git show "$tag:scripts/$script" > "$work/$script"
done
curl -fsS --max-time 30 "$site/api/health" | python3 -c \
  'import json,sys; got=json.load(sys.stdin).get("version"); want=sys.argv[1]; assert got==want, (got,want)' "$version"
bash "$work/web-entry-cache-check.sh" "$site"
note "Deployed target-tag canary: ALL GREEN"
