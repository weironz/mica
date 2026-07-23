#!/usr/bin/env bash
# Build and (re)deploy the whole stack on this machine.
#   ./deploy/deploy.sh            # full build + up
#   ./deploy/deploy.sh --web-only # only rebuild/replace the Flutter bundle
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ ! -f .env.prod ]]; then
  echo "missing .env.prod — start from deploy/.env.prod.example" >&2
  exit 1
fi

FLUTTER="${FLUTTER:-flutter}"

echo "==> building Flutter web bundle"
# --no-web-resources-cdn: without it flutter_bootstrap.js fetches CanvasKit from
# www.gstatic.com at runtime (unreachable from CN -> app breaks) and the
# canvaskit/ we ship goes dead. Keep flags in lockstep with `just build-web`.
(cd clients/mica_flutter && "$FLUTTER" build web --release --no-web-resources-cdn)
# cp -r, not rsync: this is a Bash/Linux-only fallback but rsync still isn't a
# given; a clean copy of the fresh bundle is all we need.
rm -rf deploy/web && mkdir -p deploy/web
cp -r clients/mica_flutter/build/web/. deploy/web/

if [[ "${1:-}" == "--web-only" ]]; then
  echo "==> web bundle replaced (nginx serves it live, no restart needed)"
  exit 0
fi

echo "==> building API image + starting the stack"
docker compose --env-file .env.prod -f deploy/docker-compose.single.yml build api
docker compose --env-file .env.prod -f deploy/docker-compose.single.yml up -d

echo "==> waiting for health"
for _ in $(seq 1 30); do
  if curl -fsS "http://127.0.0.1/api/health" >/dev/null 2>&1; then
    echo "==> deployed: http://$(grep -E '^SERVER_IP=' .env.prod | cut -d= -f2)/"
    exit 0
  fi
  sleep 2
done
echo "health check failed — inspect: docker compose -f deploy/docker-compose.single.yml logs api" >&2
exit 1
