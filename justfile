# Mica build / release orchestration.   `just --list` shows everything.

flutter := env_var_or_default("FLUTTER", "flutter")
hub     := "willdockerhub"

# ---------------------------------------------------------------- dev loop

# Infra only (postgres + rustfs in compose); app processes run on the host
# for fast incremental builds.
dev-up:
    docker compose up -d

dev-down:
    docker compose down

# Host-run API with dev env (fast `cargo run` cycle).
dev-api:
    bash -c 'set -a && . ./.env && set +a && cargo run -p mica-api-server'

# Rebuild the web bundle — the compose `web` (nginx) serves the bind-mounted
# build dir live; just refresh the browser afterwards. The chmod matters:
# flutter recreates build/web with 750 and the nginx container (different
# uid) 403s on the whole bundle.
dev-web:
    cd clients/mica_flutter && {{flutter}} build web --no-tree-shake-icons
    chmod -R a+rX clients/mica_flutter/build/web

# Container-parity check: run the REAL images locally before a release —
# catches container-only bugs (e.g. loopback binds) that host dev can't.
# Stops dev infra first (port clash); needs a root .env.prod.
parity-check version="dev": (docker-build version)
    docker compose down 2>/dev/null || true
    pkill -x mica-api-server 2>/dev/null || true
    MICA_VERSION={{version}} docker compose --env-file .env.prod -f deploy/docker-compose.single.yml up -d
    sleep 8
    curl -fsS http://127.0.0.1/api/health && echo " ← parity OK (stack: deploy/docker-compose.single.yml)"

# Run all tests (Rust workspace + Flutter).
test:
    cargo test --workspace
    cd clients/mica_flutter && {{flutter}} test

# Static analysis on both sides.
check:
    cargo clippy --workspace 2>/dev/null || cargo build --workspace
    cd clients/mica_flutter && {{flutter}} analyze lib

# ---------------------------------------------------------------- artifacts

# Build the Flutter web bundle and stage it for nginx/the web image.
web-build:
    cd clients/mica_flutter && {{flutter}} build web --release --no-tree-shake-icons
    mkdir -p deploy/web
    rsync -a --delete clients/mica_flutter/build/web/ deploy/web/

# Release binary for the host platform (used for GitHub release assets).
api-build:
    cargo build --release -p mica-api-server

# ---------------------------------------------------------------- docker

# Build both images, tagged {{hub}}/mica-{api,web}:<version> and :latest.
docker-build version: web-build
    docker build -f deploy/Dockerfile.api -t {{hub}}/mica-api:{{version}} -t {{hub}}/mica-api:latest .
    docker build -f deploy/Dockerfile.web -t {{hub}}/mica-web:{{version}} -t {{hub}}/mica-web:latest .

# Push a built version (and :latest) to Docker Hub.
docker-push version:
    docker push {{hub}}/mica-api:{{version}}
    docker push {{hub}}/mica-api:latest
    docker push {{hub}}/mica-web:{{version}}
    docker push {{hub}}/mica-web:latest

# ---------------------------------------------------------------- release

# Tarball release assets: the linux api binary + the web bundle.
release-assets version: api-build web-build
    rm -rf dist && mkdir -p dist
    tar -C target/release -czf dist/mica-api-server-{{version}}-linux-x86_64.tar.gz mica-api-server
    tar -C deploy -czf dist/mica-web-{{version}}.tar.gz web
    ls -lh dist

# Full release: tests, images to Docker Hub, git tag, GitHub release with
# binaries (needs `docker login` and `gh auth login` done once).
release version: test (docker-build version) (docker-push version) (release-assets version)
    git tag -f {{version}}
    git push origin {{version}}
    gh release create {{version}} dist/* \
      --title "Mica {{version}}" \
      --notes-file deploy/release-notes-{{version}}.md

# ---------------------------------------------------------------- deploy

# Single-server production deploy (see docs/deploy.md).
deploy:
    ./deploy/deploy.sh

deploy-web:
    ./deploy/deploy.sh --web-only
