#!/usr/bin/env bash
# Bump the three places the version lives. Called by scripts/release.sh; running
# it alone is fine but then the gate is back to being something you remember.
set -euo pipefail
[ -d "/c/Program Files/Git/usr/bin" ] && PATH="/c/Program Files/Git/usr/bin:$PATH"

version=$1
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "REFUSED: want X.Y.Z" >&2; exit 1; }
old=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
[ "$old" != "$version" ] || { echo "REFUSED: already at $version" >&2; exit 1; }

# Three files, one source of truth would be better — see docs/cd-plan.md §2.
# Until that lands, at least stop hand-editing three places in three formats.
sed -i "s/^version: $old/version: $version/" clients/mica_flutter/pubspec.yaml
sed -i "s/kAppVersion = '$old'/kAppVersion = '$version'/" clients/mica_flutter/lib/main.dart
sed -i "0,/^version = \"$old\"/s//version = \"$version\"/" Cargo.toml
cargo check -p mica-api-server >/dev/null   # refreshes Cargo.lock

echo "==> $old -> $version (pubspec / kAppVersion / Cargo.toml / Cargo.lock)"
