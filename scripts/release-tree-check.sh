#!/usr/bin/env bash
# Shared pre-tag checks for the local and hosted release paths (100% pure Rust).
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=$(bash scripts/manifest-version.sh)
bash scripts/check-migration-tracking.sh

for package in mica-api-server mica-cli; do
  lock=$(sed -n "/^name = \"$package\"$/{n;s/^version = \"\(.*\)\"$/\1/p;}" Cargo.lock)
  [ "$lock" = "$version" ] || fail "Cargo.lock: $package is '$lock', expected '$version'"
done

client_lock=$(sed -n "/^name = \"mica-rust\"$/{n;s/^version = \"\(.*\)\"$/\1/p;}" clients/mica_rust/Cargo.lock)
[ "$client_lock" = "$version" ] || fail "clients/mica_rust/Cargo.lock: mica-rust is '$client_lock', expected '$version'"

for doc in README.md docs/ops/deploy.md; do
  pinned=$(sed -nE 's/.*RELEASE=([0-9]+\.[0-9]+\.[0-9]+).*/\1/p' "$doc" | sed -n '1p')
  [ "$pinned" = "$version" ] || fail "$doc pins the quickstart to '$pinned', expected '$version'"
done

rust_pin=$(sed -n 's/^channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)
[ -n "$rust_pin" ] || fail "could not read rust-toolchain.toml"
for image in deploy/Dockerfile.api deploy/Dockerfile.cli docker-compose.yml; do
  grep -Fq "rust:${rust_pin%.*}-slim-bookworm" "$image" \
    || fail "$image must build with the Rust pin $rust_pin"
done

shipped=$(git tag --list 'v[0-9]*')
[ -n "$shipped" ] || fail "no release tags; fetch the full history and tags before checking migrations"
shipped_paths=$(git log --tags='v[0-9]*' --format= --name-only --diff-filter=A -- migrations/ | sed '/^$/d' | sort -u)
while IFS= read -r file; do
  [ -n "$file" ] || continue
  added=$(git log --all --diff-filter=A --format=%H -- "$file" | tail -1)
  [ -n "$added" ] || continue
  first=$(git tag --contains "$added" --list 'v[0-9]*' --sort=v:refname | sed -n '1p')
  [ -n "$first" ] || continue
  git diff --quiet "$first" -- "$file" \
    || fail "$file differs from $first; keep the shipped bytes and add a new migration"
done <<< "$shipped_paths"

echo "release tree checks passed for $version"
