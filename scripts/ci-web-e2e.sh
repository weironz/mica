#!/usr/bin/env bash
# The `web-e2e` job's nginx container: serve | stop.
#
# nginx, not a plain static server: two of that job's assertions are ABOUT
# nginx — that /verify-email, /reset-password and /s/ beat the SPA fallback, and
# that the fixed-name entry files are not cacheable. A different server would
# test a routing table nobody deploys. The dev config points at
# host.docker.internal, which resolves on the runner via --add-host.
#
# The container keeps a FIXED name and a FIXED port on purpose: the harness, the
# failure log and every "docker logs mica-web-e2e" instruction name them, and a
# run-scoped name would only hide which run left it behind. It is therefore not
# concurrency-safe on one daemon — a self-hosted runner runs one job at a time —
# and it must be removed by the job that started it, on every path (the workflow
# calls `stop` from an `if: always()` step, after the failure log).
set -euo pipefail

name=${MICA_CI_WEB_CONTAINER:-mica-web-e2e}
bundle=clients/mica_flutter/build/web

case "${1:-}" in
  serve)
    # BEFORE nginx starts: flutter writes build/web mode 750 and the container
    # runs a different uid, so it would 403 on everything.
    chmod -R a+rX "$bundle"
    # `/etc/nginx/nginx.conf`, NOT conf.d/default.conf — deploy/nginx.dev.conf is
    # a whole nginx config (`worker_processes`, `events`, `http { … }`), not a
    # snippet, and nginx refuses to start if it is dropped into conf.d.
    docker run -d --name "$name" \
      --add-host host.docker.internal:host-gateway \
      -p 8090:80 \
      -v "$PWD/deploy/nginx.dev.conf:/etc/nginx/nginx.conf:ro" \
      -v "$PWD/$bundle:/usr/share/nginx/html:ro" \
      nginx:alpine
    # Fail here if the config is bad, rather than 60s later as "nginx cannot
    # reach the api" — a wrong diagnosis costs more than the wait.
    sleep 3
    docker exec "$name" nginx -t
    ;;
  stop)
    # Only this fixed name is ever removed; a developer's stack (and GitHub's
    # managed `services:` containers) are untouched. Absent container is not an
    # error: the job can fail before `serve` runs, and the cleanup still must
    # succeed then.
    docker rm -f "$name" 2>/dev/null || true
    ;;
  *)
    echo "usage: $0 serve|stop" >&2
    exit 2
    ;;
esac