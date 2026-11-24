#!/usr/bin/env bash
# Bump every place the version is written down: root Cargo.toml,
# clients/mica_rust/Cargo.toml, both Cargo.lock files, Windows installer script,
# and quickstart docs pinning `RELEASE=`.
set -euo pipefail
[ -d "/c/Program Files/Git/usr/bin" ] && PATH="/c/Program Files/Git/usr/bin:$PATH"

version=$1
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "REFUSED: want X.Y.Z" >&2; exit 1; }
old=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
[ "$old" != "$version" ] || { echo "REFUSED: already at $version" >&2; exit 1; }

sed -i "0,/^version = \"$old\"/s//version = \"$version\"/" Cargo.toml
sed -i "0,/^version = \"$old\"/s//version = \"$version\"/" clients/mica_rust/Cargo.toml
sed -i "s/#define AppVersion \"$old\"/#define AppVersion \"$version\"/" clients/mica_rust/installer/mica.iss
cargo update --workspace
cargo update --manifest-path clients/mica_rust/Cargo.toml --workspace

for doc in README.md docs/ops/deploy.md; do
  [ -f "$doc" ] || { echo "REFUSED: $doc is missing — the quickstart gate reads its RELEASE= pin" >&2; exit 1; }
  grep -q "RELEASE=$old" "$doc" || { echo "REFUSED: $doc has no RELEASE=$old pin to move to $version" >&2; exit 1; }
  sed -i "0,/RELEASE=$old/s//RELEASE=$version/" "$doc"
done

echo "==> $old -> $version (Cargo.toml / clients/mica_rust/Cargo.toml / Cargo.lock / mica.iss / README.md / docs/ops/deploy.md)"
