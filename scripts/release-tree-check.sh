#!/usr/bin/env bash
# Shared pre-tag checks for the local and hosted release paths.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=$(bash scripts/manifest-version.sh)
bash scripts/check-migration-tracking.sh

for package in mica-api-server mica-cli; do
  lock=$(sed -n "/^name = \"$package\"$/{n;s/^version = \"\(.*\)\"$/\1/p;}" Cargo.lock)
  [ "$lock" = "$version" ] || fail "Cargo.lock: $package is '$lock', expected '$version'"
done

for doc in README.md docs/ops/deploy.md; do
  pinned=$(sed -nE 's/.*RELEASE=([0-9]+\.[0-9]+\.[0-9]+).*/\1/p' "$doc" | sed -n '1p')
  [ "$pinned" = "$version" ] || fail "$doc pins the quickstart to '$pinned', expected '$version'"
done

cmp -s clients/mica_flutter/assets/tray_icon.ico clients/mica_flutter/windows/runner/resources/app_icon.ico \
  || fail "tray icon and Windows app icon must be byte-identical"

rust_pin=$(sed -n 's/^channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)
[ -n "$rust_pin" ] || fail "could not read rust-toolchain.toml"
for image in deploy/Dockerfile.api deploy/Dockerfile.cli docker-compose.yml; do
  grep -Fq "rust:${rust_pin%.*}-slim-bookworm" "$image" \
    || fail "$image must build with the Rust pin $rust_pin"
done

client=$(sed -nE 's/.*postgresql-client-([0-9]+).*/\1/p' deploy/Dockerfile.cli | sed -n '1p')
server=$(sed -nE 's/.*image: postgres:([0-9]+).*/\1/p' deploy/docker-compose.yml | sed -n '1p')
[[ -n "$client" && -n "$server" && "$client" -ge "$server" ]] \
  || fail "pg_dump client ('$client') must be at least the deployed PostgreSQL major ('$server')"

# The first release containing each migration fixes its exact bytes, comments
# included. Take the union of shipped paths so deletion is rejected too.
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
