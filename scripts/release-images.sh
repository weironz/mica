#!/usr/bin/env bash
# Build mica-api / mica-web / mica-cli images for vX.Y.Z and push them to BOTH
# registries prod and its off-site copy read from — the `images` job of
# release.yml, on this machine.
#
#   just release-images 0.13.57
#   PUSH=0 just release-images 0.13.57     # build + tag only
#
# Needs Docker (Linux, or Docker Desktop on Windows) and, to push,
#   docker login registry.cn-shenzhen.aliyuncs.com
#   docker login                                   (Docker Hub)
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

note "web bundle (flutter build web)"
(cd clients/mica_flutter && "$FLUTTER" pub get >/dev/null && "$FLUTTER" build web --release --no-web-resources-cdn)
bash clients/mica_flutter/tool/gfonts/mirror.sh --check
rm -rf deploy/web && mkdir -p deploy/web
cp -r clients/mica_flutter/build/web/. deploy/web/

for img in api web cli; do
  note "build mica-$img:v$version"
  # --provenance/--sbom off: the attestation manifest turns the image into a
  # multi-manifest index some registries / `docker save | load` cannot resolve.
  docker build --provenance=false --sbom=false -f "deploy/Dockerfile.$img" \
    -t "$acr/mica-$img:v$version" -t "$hub/mica-$img:v$version" .
done

if [ "${PUSH:-1}" = 1 ]; then
  for img in api web cli; do
    note "push mica-$img:v$version"
    docker push "$acr/mica-$img:v$version"
    docker push "$hub/mica-$img:v$version"
  done
else
  note "PUSH=0: built and tagged, not pushed"
fi
