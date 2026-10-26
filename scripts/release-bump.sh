#!/usr/bin/env bash
# Bump every place the version is written down: the three code manifests, the
# lock, and the two quickstart docs that pin a `RELEASE=` tag. Called by
# scripts/release.sh; running it alone is fine but then the gate is back to being
# something you remember.
set -euo pipefail
[ -d "/c/Program Files/Git/usr/bin" ] && PATH="/c/Program Files/Git/usr/bin:$PATH"

version=$1
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "REFUSED: want X.Y.Z" >&2; exit 1; }
old=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
[ "$old" != "$version" ] || { echo "REFUSED: already at $version" >&2; exit 1; }

# Three files, one source of truth would be better — see docs/ops/cd-plan.md §2.
# Until that lands, at least stop hand-editing three places in three formats.
sed -i "s/^version: $old/version: $version/" clients/mica_flutter/pubspec.yaml
sed -i "s/kAppVersion = '$old'/kAppVersion = '$version'/" clients/mica_flutter/lib/main.dart
sed -i "0,/^version = \"$old\"/s//version = \"$version\"/" Cargo.toml
cargo check -p mica-api-server >/dev/null   # refreshes Cargo.lock

# The quickstart's own version pin belongs to the bump, not to the human running
# it. release-check REFUSES a release whose docs still pin the previous
# `RELEASE=`, so leaving these to a separate manual commit meant a clean-tree
# `just release N+1` failed its own gate and left a dirty tree that blocked the
# retry (#125). First occurrence only, mirroring the gate's `head -1` read.
for doc in README.md docs/ops/deploy.md; do
  [ -f "$doc" ] || { echo "REFUSED: $doc is missing — the quickstart gate reads its RELEASE= pin" >&2; exit 1; }
  grep -q "RELEASE=$old" "$doc" || { echo "REFUSED: $doc has no RELEASE=$old pin to move to $version" >&2; exit 1; }
  sed -i "0,/RELEASE=$old/s//RELEASE=$version/" "$doc"
done

echo "==> $old -> $version (pubspec / kAppVersion / Cargo.toml / Cargo.lock / README.md / docs/ops/deploy.md)"
