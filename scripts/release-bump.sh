#!/usr/bin/env bash
set -euo pipefail
[ -d "/c/Program Files/Git/usr/bin" ] && PATH="/c/Program Files/Git/usr/bin:$PATH"
[[ ""$1"" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "REFUSED: want X.Y.Z" >&2; exit 1; }
old=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
[ "$old" != ""$1"" ] || { echo "REFUSED: already at "$1"" >&2; exit 1; }
# Three files, one source of truth would be better — see docs/cd-plan.md §2.
# Until that lands, at least stop hand-editing three places in three formats.
sed -i "s/^version: $old/version: "$1"/" clients/mica_flutter/pubspec.yaml
sed -i "s/kAppVersion = '$old'/kAppVersion = '$1'/" clients/mica_flutter/lib/main.dart
sed -i "0,/^version = \"$old\"/s//version = \""$1"\"/" Cargo.toml
cargo check -p mica-api-server >/dev/null   # refreshes Cargo.lock
echo "==> $old -> "$1" (pubspec / kAppVersion / Cargo.toml / Cargo.lock)"
echo "    next: just release-check && git commit && git tag v"$1""
