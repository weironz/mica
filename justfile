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

# Production node. Prod pulls its images from Aliyun ACR (first-party, fast in
# CN); CI pushes there on every `v*` tag. `deploy-prod` is the ONLY recipe that
# touches prod — CI never does (docs/release.md explains why).
node     := "root@mica.cloudcele.com"
node_dir := "/data/mica"
site     := "https://mica.cloudcele.com"

# Registry for LOCAL image builds (parity-check / a CI outage fallback).
# Prod's own registry lives in {{node_dir}}/.env as MICA_REGISTRY.
hub_acr  := "registry.cn-shenzhen.aliyuncs.com/willspace"

# ---------------------------------------------------------------- dev loop

# One command in, one command out. The API is the `api` container (bind-mounted
# source, cargo cache in volumes), so `dev-down` really does stop everything.
#
# There used to be a second way — `dev-up` (infra only) plus `dev-api`
# (cargo run on the host) — kept around as an escape hatch. It earned nothing:
# the container rebuilds a one-line change in ~5 s, so the host had no speed
# left to offer, while `dev-down` silently failed to stop it and left :8080 held
# by a process nothing in the stack accounted for. Two ways to start the backend,
# one of them booby-trapped, is worse than one way.
#
# First run compiles the workspace into the volume (~5.5 min here); after that a
# one-line change rebuilds and restarts in ~5 s.
[doc("Start the whole dev stack (infra + API + web) and seed it")]
dev:
    docker compose up -d
    @echo "waiting for the API (first run compiles the workspace — minutes)"
    @for i in $(seq 1 200); do \
        curl -fsS http://127.0.0.1:8080/api/health >/dev/null 2>&1 && break; \
        sleep 3; \
    done
    @curl -fsS http://127.0.0.1:8080/api/health || (echo "API never came up — docker compose logs api" && exit 1)
    @echo ""
    just seed-dev
    @echo "api http://127.0.0.1:8080  web http://127.0.0.1:8090  (demo@mica.dev / password123)"

[doc("Tail the dev API log (it runs in a container now, not your terminal)")]
dev-logs:
    docker compose logs -f api

[doc("Stop the whole dev stack (add -v to also wipe the database volume)")]
dev-down:
    docker compose down

# Run AFTER the API has started once, so sqlx::migrate! has created the tables.
# Idempotent — safe to re-run after `docker compose down -v`.
[doc("Seed a local demo account (demo@mica.dev / password123) + workspace")]
seed-dev:
    docker exec -i mica-postgres psql -U mica -d mica < seeds/dev_seed.sql

# The compose `web` (nginx) serves the bind-mounted build dir live; just
# refresh the browser afterwards. The chmod matters: flutter recreates
# build/web with 750 and the nginx container (different uid) 403s on it.
#
# SAME FLAGS AS `build-web`, deliberately. This used to pass
# --no-tree-shake-icons while the shipped bundle (build-web) tree-shakes, so the
# thing you eyeballed at :8090 was not the thing you released: icon glyphs that
# get stripped in production were all present here. The whole point of this
# target is to look at the real artifact, so any flag that diverges from
# build-web defeats it — keep the two lines identical.
[doc("Rebuild the dev web bundle served by compose nginx")]
dev-web:
    cd clients/mica_flutter && {{flutter}} build web --release
    chmod -R a+rX clients/mica_flutter/build/web

# Hot reload: press r; quit: q. Desktop opens the offline local world —
# create folders/pages with no backend. MICA_DEV_AUTOLOGIN=false because this
# recipe runs no backend: leaving it on makes startup try (and fail) to sign in
# a demo account against 127.0.0.1:8080, dumping a raw connection error banner.
#   just app          # desktop (windows)
#   just app chrome   # web
[doc("Launch the app to eyeball a change (target: windows | chrome)")]
app target="windows":
    cd clients/mica_flutter && {{flutter}} run -d {{target}} --dart-define=MICA_DEV_AUTOLOGIN=false

# Catches container-only bugs (e.g. loopback binds) that host dev can't.
# Stops the dev stack first — both bind :80/:9000/:5432.
#
# Two things used to keep this from running at all:
#
# 1. It read `.env.prod`, the file holding REAL production credentials, so a
#    local check demanded a full production secret set. Nobody set that up, so
#    the check nobody could run also caught nothing. Now it reads
#    deploy/.env.parity — committed throwaway values.
# 2. It depended on `docker-build`, which built three TAGGED images that
#    single.yml never looks at: that file `build:`s the api from
#    deploy/Dockerfile.api and contains no ${MICA_VERSION} at all. The only
#    thing it needed from that chain was `build-web`, which stages deploy/web
#    for the nginx mount — so depend on that directly and skip building two
#    images for nothing.
[doc("Run the prod container stack locally before a release (container parity)")]
parity-check: build-web
    docker compose down 2>/dev/null || true
    docker compose --project-directory . --env-file deploy/.env.parity \
        -f deploy/docker-compose.single.yml up -d --build
    @echo "waiting for the prod stack on :80"
    @for i in $(seq 1 60); do \
        curl -fsS http://127.0.0.1/api/health >/dev/null 2>&1 && break; \
        sleep 3; \
    done
    @curl -fsS http://127.0.0.1/api/health \
        && echo " <- parity OK (deploy/docker-compose.single.yml)" \
        || (echo "parity FAILED — docker compose -f deploy/docker-compose.single.yml logs" && exit 1)
    @echo "stop it with:  just parity-down"

[doc("Stop the parity stack (and free :80 for the dev stack)")]
parity-down:
    docker compose --project-directory . --env-file deploy/.env.parity \
        -f deploy/docker-compose.single.yml down

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

# Release images come from CI; this is for `parity-check` and as a fallback
# if CI is down. --provenance/--sbom off: buildx defaults attach an OCI
# attestation manifest, turning the image into a multi-manifest index that
# some registries / `docker save | docker load` cannot resolve.
[doc("Build the prod images locally (parity-check / CI-outage fallback)")]
docker-build tag: build-web
    docker build --provenance=false --sbom=false -f deploy/Dockerfile.api -t {{hub_acr}}/mica-api:{{tag}} .
    docker build --provenance=false --sbom=false -f deploy/Dockerfile.web -t {{hub_acr}}/mica-web:{{tag}} .
    docker build --provenance=false --sbom=false -f deploy/Dockerfile.cli -t {{hub_acr}}/mica-cli:{{tag}} .

# Normally CI does this. Needs `docker login registry.cn-shenzhen.aliyuncs.com`.
[doc("Push locally built images to ACR (CI-outage fallback)")]
docker-push tag:
    docker push {{hub_acr}}/mica-api:{{tag}}
    docker push {{hub_acr}}/mica-web:{{tag}}
    docker push {{hub_acr}}/mica-cli:{{tag}}

# ---------------------------------------------------------------- deploy

# Prod pulls the CI-built images from ACR and restarts. --no-deps keeps
# postgres / rustfs untouched. api, web AND the backup sidecar (mica-cli) all
# roll to <version> — CI publishes the three keyed to the same MICA_VERSION, so
# a deploy must move all three or backup silently drifts (it sat on the old
# willdockerhub/mica-cli:v0.3 for many releases because deploy skipped it).
# MICA_VERSION is rewritten in the node's .env so a restart (or a reboot) comes
# back on the SAME version, not the old one.
# Install the node-side deploy policy. ROOT-SIDE AND MANUAL ON PURPOSE.
#
# `/usr/local/sbin/mica-deploy` is the fence that limits what the CI key can do,
# so CI must never be able to write it — anything that can rewrite the fence is
# not constrained by it. Same reason `authorized_keys` and `sudoers` are hand-
# installed. Run this yourself, with your own root key, when the script changes.
#
# From the TAG, not the working tree: the first installs of this script were
# `scp` straight from a dirty checkout — exactly the drift `deploy-prod` was
# just fixed to avoid. Writes to `.new`, syntax-checks, then moves into place,
# so a truncated transfer can never leave a half-written policy behind.
# Defaults to `main`, not to a release tag: the deploy policy and the application
# version are independent timelines. Pinning it to a tag also breaks for every
# tag older than the script itself — v0.12.8 has no deploy/mica-deploy.sh at all.
[doc("Install deploy/mica-deploy.sh on the node from a ref (root, manual)")]
sync-deploy-script ref="origin/main":
    #!/usr/bin/env bash
    set -euo pipefail
    tag="{{ref}}"
    git fetch -q origin main
    git show "$tag:deploy/mica-deploy.sh" \
      | ssh {{node}} "cat > /usr/local/sbin/mica-deploy.new \
          && chown root:root /usr/local/sbin/mica-deploy.new \
          && chmod 0755 /usr/local/sbin/mica-deploy.new \
          && bash -n /usr/local/sbin/mica-deploy.new \
          && mv /usr/local/sbin/mica-deploy.new /usr/local/sbin/mica-deploy \
          && sha256sum /usr/local/sbin/mica-deploy"
    echo "want: $(git show "$tag:deploy/mica-deploy.sh" | sha256sum)"

[doc("Roll prod to an already-published version, e.g. `just deploy-prod 0.5.1`")]
deploy-prod version:
    #!/usr/bin/env bash
    set -euo pipefail
    tag="v{{version}}"
    # The node's compose is NOT a git checkout — it was hand-placed and drifted:
    # prod sat on a hardcoded willdockerhub/... image while the repo had already
    # moved to ACR, so a "deploy" silently kept pulling from the old registry.
    # Ship the repo's copy every time (backing up first) — repo is the truth.
    # NB the node's file is docker-compose.yaml; keep that name, or compose
    # would find two files and pick the other one.
    # (No backticks anywhere in a recipe body: just runs them as commands,
    # even inside a # comment.)
    # Take the compose from the TAG, not the working tree. `scp deploy/...`
    # shipped whatever happened to be checked out: deploying 0.12.6 from a
    # branch that had already moved on sent prod a compose from a DIFFERENT
    # version, and a dirty tree sent uncommitted edits. Nothing announced it —
    # the deploy just succeeded with a file nobody reviewed as part of that
    # release. `git show <tag>:<path>` makes the pairing exact and reproducible
    # from any checkout state.
    if ! git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
      echo "==> tag $tag not local, fetching"
      git fetch -q origin "refs/tags/$tag:refs/tags/$tag"
    fi
    echo "==> syncing compose (from $tag) + pinning MICA_VERSION=$tag"
    git show "$tag:deploy/docker-compose.yml" \
      | ssh {{node}} "cat > {{node_dir}}/docker-compose.yaml.new"
    ssh {{node}} "cd {{node_dir}} \
      && cp docker-compose.yaml docker-compose.yaml.bak-\$(date +%Y%m%d-%H%M%S) \
      && mv docker-compose.yaml.new docker-compose.yaml \
      && sed -i -E 's|^MICA_VERSION=.*|MICA_VERSION=$tag|' .env \
      && grep -E '^MICA_(VERSION|REGISTRY)=' .env"
    echo "==> pulling + recreating api + web"
    ssh {{node}} "cd {{node_dir}} && docker compose pull api web && docker compose up -d --no-deps api web"
    # The backup sidecar (mica-cli: exports the workspace + external rustic
    # snapshots it) is keyed to the SAME MICA_VERSION, so a version roll must
    # move it too or it drifts. It lives behind the `backup` compose profile;
    # only refresh it where it is ALREADY present (ps -aq returns a container),
    # so a deploy never switches backup ON on a node that runs without it.
    echo "==> refreshing backup sidecar (only if this node runs it)"
    ssh {{node}} "cd {{node_dir}} \
      && if [ -n \"\$(docker compose --profile backup ps -aq backup 2>/dev/null)\" ]; then \
           docker compose --profile backup pull backup \
           && docker compose --profile backup up -d --no-deps backup \
           && echo 'backup refreshed'; \
         else echo 'backup profile not active on this node — skipped'; fi"
    # Go-template braces vs just: four open-braces escape a literal two, but a
    # closing pair is ALREADY literal outside an interpolation — writing four
    # closers emitted two extra, so the format returned "healthy}}" and the
    # compare never matched. It cried "NOT healthy" over a healthy deploy.
    echo "==> waiting for api health"
    ssh {{node}} 'for i in $(seq 1 60); do s=$(docker inspect --format "{{{{.State.Health.Status}}" mica-api-1 2>/dev/null || true); [ "$s" = healthy ] && { echo "api healthy"; exit 0; }; sleep 4; done; echo "api NOT healthy (last state: ${s:-unknown})"; exit 1'
    just verify-prod {{version}}
    echo "==> prod now on $tag from the registry in {{node_dir}}/.env"

# The api must report the version we just rolled to, and the live bundle must
# not be a cached/stale artifact. Checking only for HTTP 200 would miss both.
[doc("Prove prod really serves <version>, e.g. `just verify-prod 0.5.1`")]
verify-prod version:
    #!/usr/bin/env bash
    set -euo pipefail
    got=$(curl -fsS {{site}}/api/health)
    echo "$got"
    echo "$got" | grep -q '"version":"{{version}}"' \
      || { echo "VERSION MISMATCH: prod is not {{version}}"; exit 1; }
    echo "mcp:   $(curl -s -o /dev/null -w '%{http_code}' {{site}}/mcp)"
    echo "index: $(curl -s -o /dev/null -w '%{http_code}' {{site}}/)"
    echo "bundle: $(curl -fsS {{site}}/main.dart.js | md5sum | cut -c1-12)…"
