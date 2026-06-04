# Mica build / release orchestration.   `just --list` shows everything.

flutter := env_var_or_default("FLUTTER", "flutter")
hub     := "willdockerhub"

# ---------------------------------------------------------------- dev loop

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
