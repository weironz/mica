# Mica build / release orchestration.   `just --list` shows everything.
# Full flow, prerequisites and rationale: docs/release.md
#
# Recipes are POSIX and run under bash. On Windows that MUST be Git Bash —
# the `bash` on PATH is C:\WINDOWS\system32\bash.exe (the WSL launcher), and
# WSL has none of the Windows toolchain (docker / flutter / cargo) and a
# different filesystem view (/mnt/d/...). `windows-shell` pins the right one.
set shell := ["bash", "-uc"]
set windows-shell := ["C:/Program Files/Git/bin/bash.exe", "-uc"]

flutter := env_var_or_default("FLUTTER", "flutter")
hub     := "willdockerhub"

# Production node. It CANNOT reach Docker Hub, so images travel by
# save|gzip -> scp -> docker load (see `ship`); the Hub push is a separate
# off-site copy, not the delivery path.
node     := "root@mica.cloudcele.com"
node_dir := "/data/mica"
site     := "https://mica.cloudcele.com"

# The rolling DEPLOY tag compose reads from {{node_dir}}/.env (MICA_VERSION).
# It is a pointer, NOT the app version — v0.3 has rolled through every 0.3–0.5
# release. Changing it means editing .env on the node AND retagging mica-cli.
deploy_tag := "v0.3"

# ---------------------------------------------------------------- dev loop

# Infra only (postgres + rustfs in compose); app processes run on the host
# for fast incremental builds.
[doc("Start dev infra (postgres + rustfs) in compose")]
dev-up:
    docker compose up -d

[doc("Stop dev infra")]
dev-down:
    docker compose down

[doc("Run the API on the host against .env (fast cargo cycle)")]
dev-api:
    set -a && . ./.env && set +a && cargo run -p mica-api-server

# The compose `web` (nginx) serves the bind-mounted build dir live; just
# refresh the browser afterwards. The chmod matters: flutter recreates
# build/web with 750 and the nginx container (different uid) 403s on it.
[doc("Rebuild the dev web bundle served by compose nginx")]
dev-web:
    cd clients/mica_flutter && {{flutter}} build web --no-tree-shake-icons
    chmod -R a+rX clients/mica_flutter/build/web

# Hot reload: press r; quit: q. Desktop opens the offline local world —
# create folders/pages with no backend.
#   just app          # desktop (windows)
#   just app chrome   # web
[doc("Launch the app to eyeball a change (target: windows | chrome)")]
app target="windows":
    cd clients/mica_flutter && {{flutter}} run -d {{target}}

# Catches container-only bugs (e.g. loopback binds) that host dev can't.
# Stops dev infra first (port clash); needs a root .env.prod.
[doc("Run the REAL images locally before a release (container parity)")]
parity-check tag="dev": (docker-build tag)
    docker compose down 2>/dev/null || true
    MICA_VERSION={{tag}} docker compose --env-file .env.prod -f deploy/docker-compose.single.yml up -d
    sleep 8
    curl -fsS http://127.0.0.1/api/health && echo " <- parity OK (deploy/docker-compose.single.yml)"

[doc("Run all tests (Rust workspace + Flutter)")]
test:
    cargo test --workspace
    cd clients/mica_flutter && {{flutter}} test

[doc("Static analysis on both sides")]
check:
    cargo clippy --workspace 2>/dev/null || cargo build --workspace
    cd clients/mica_flutter && {{flutter}} analyze lib

# ------------------------------------------------------- local artifacts
# The four things a release ships. 1+2 are also built by GitHub Actions on a
# `v*` tag (docs/release.md); these recipes are for building them by hand.

[doc("1/4  mica-cli binary for this host -> target/release/")]
build-cli:
    cargo build --locked -p mica-cli --release
    @echo "-> target/release/mica-cli"

# Needs Inno Setup:  choco install innosetup -y
# Windows only (ISCC.exe); CI does this on windows-latest.
[doc("2/4  Windows desktop installer -> installer/Output/ (needs Inno Setup)")]
build-installer version:
    cd clients/mica_flutter && {{flutter}} build windows --release
    "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" //DAppVersion={{version}} 'clients\mica_flutter\installer\mica.iss'
    @echo "-> clients/mica_flutter/installer/Output/Mica-Setup-{{version}}.exe"

# `cp -r`, not rsync: Windows has no rsync (the old recipe just failed).
[doc("3/4  Flutter web bundle -> deploy/web (staged for the web image)")]
build-web:
    cd clients/mica_flutter && {{flutter}} build web --release
    rm -rf deploy/web && mkdir -p deploy/web
    cp -r clients/mica_flutter/build/web/. deploy/web/
    @echo "-> deploy/web (main.dart.js $(md5sum deploy/web/main.dart.js | cut -c1-12)…)"

# The DEPLOYED api is the docker image below; this binary is for local
# runs / profiling.
[doc("4/4  api-server binary for this host -> target/release/")]
build-api:
    cargo build --release -p mica-api-server
    @echo "-> target/release/mica-api-server"

[doc("Build cli + web + api (everything without a Windows-only toolchain)")]
build-all: build-cli build-web build-api

# ---------------------------------------------------------------- docker

# --provenance/--sbom off: buildx defaults attach an OCI attestation
# manifest, turning the image into a multi-manifest index that
# `docker save | docker load` on the node cannot resolve.
[doc("Build the two prod images (api + web) locally")]
docker-build tag=deploy_tag: build-web
    docker build --provenance=false --sbom=false -f deploy/Dockerfile.api -t {{hub}}/mica-api:{{tag}} .
    docker build --provenance=false --sbom=false -f deploy/Dockerfile.web -t {{hub}}/mica-web:{{tag}} .

# Needs `docker login` once. This is the off-site copy — the node is fed by
# `ship`, which does not pull from Hub.
[doc("Push the api + web images to Docker Hub")]
docker-push tag=deploy_tag:
    docker push {{hub}}/mica-api:{{tag}}
    docker push {{hub}}/mica-web:{{tag}}

# ---------------------------------------------------------------- deploy

# save|gzip|ssh docker load, because the node has no Docker Hub access.
# --no-deps so postgres / rustfs / backup keep running untouched.
[doc("Copy the built images to the prod node and recreate api + web")]
ship tag=deploy_tag:
    #!/usr/bin/env bash
    set -euo pipefail
    for img in api web; do
      echo "==> shipping mica-$img:{{tag}}"
      docker save {{hub}}/mica-$img:{{tag}} | gzip -1 | \
        ssh {{node}} "gunzip -c | docker load"
    done
    ssh {{node}} 'cd {{node_dir}} && docker compose up -d --force-recreate --no-deps api web'
    echo "==> recreated; waiting for api health"
    ssh {{node}} 'for i in $(seq 1 30); do [ "$(docker inspect --format "{{{{.State.Health.Status}}}}" mica-api-1 2>/dev/null)" = healthy ] && { echo "api healthy"; exit 0; }; sleep 4; done; echo "api NOT healthy"; exit 1'

# The live bundle hash must equal the local one (catches a stale image /
# cached layer), plus the api version and the MCP endpoint.
[doc("Prove prod serves exactly what was just built")]
verify-prod:
    #!/usr/bin/env bash
    set -euo pipefail
    want=$(md5sum deploy/web/main.dart.js | cut -d' ' -f1)
    live=$(curl -fsS {{site}}/main.dart.js | md5sum | cut -d' ' -f1)
    [ "$want" = "$live" ] && echo "bundle OK  $live" || { echo "STALE BUNDLE: want $want, live $live"; exit 1; }
    curl -fsS {{site}}/api/health; echo
    echo "mcp: $(curl -s -o /dev/null -w '%{http_code}' {{site}}/mcp)"
    echo "index: $(curl -s -o /dev/null -w '%{http_code}' {{site}}/)"

# build images -> ship to the node -> push to Hub -> verify. The desktop
# installer and mica-cli binaries are NOT here — GitHub Actions builds those
# when you push the `v*` tag (docs/release.md).
[doc("The whole web+api release: build -> ship -> push -> verify")]
deploy-prod tag=deploy_tag: (docker-build tag) (ship tag) (docker-push tag) verify-prod
    @echo "==> web + api live at {{site}}"
