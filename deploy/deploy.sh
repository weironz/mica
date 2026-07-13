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
(cd clients/mica_flutter && "$FLUTTER" build web --release --no-tree-shake-icons)
mkdir -p deploy/web
rsync -a --delete clients/mica_flutter/build/web/ deploy/web/

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
