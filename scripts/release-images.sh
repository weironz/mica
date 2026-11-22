#!/usr/bin/env bash
# Build mica-api / mica-web / mica-cli images for vX.Y.Z and push them to ACR + Docker Hub.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=${1:-}
check_version_arg "$version"
require_tagged_tree "$version"
bash scripts/check-migration-tracking.sh
docker info >/dev/null 2>&1 || fail "Docker is not running"

acr=registry.cn-shenzhen.aliyuncs.com/willspace
hub=willdockerhub

note "pure-Rust WebAssembly bundle (clients/mica_rust -> deploy/web)"
bash scripts/build-web-rust.sh

for img in api web cli; do
  note "build mica-$img:v$version"
  docker build --provenance=false --sbom=false -f "deploy/Dockerfile.$img" \
    -t "$acr/mica-$img:v$version" -t "$hub/mica-$img:v$version" .
done

if [ "${PUSH:-1}" = 1 ]; then
  for img in api web cli; do
    note "push mica-$img:v$version"
    docker push "$acr/mica-$img:v$version"
    docker push "$hub/mica-$img:v$version"
  done
  out=$(dist_dir "$version")
  mkdir -p "$out"
  git rev-parse HEAD > "$out/.images-pushed"
else
  note "PUSH=0: built and tagged, not pushed"
fi
