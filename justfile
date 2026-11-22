# Mica build / release orchestration (100% Fullstack Rust).
# `just --list` shows everything. Full flow: docs/ops/release.md
set shell := ["bash", "-uc"]
set windows-shell := ["C:/Program Files/Git/bin/bash.exe", "-uc"]

hub      := "willdockerhub"
node     := "root@mica.cloudcele.com"
node_dir := "/data/mica"
site     := "https://mica.cloudcele.com"
hub_acr  := "registry.cn-shenzhen.aliyuncs.com/willspace"

# ---------------------------------------------------------------- dev loop

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

[doc("Tail the dev API log")]
dev-logs:
    docker compose logs -f api

[doc("Stop the whole dev stack (add -v to also wipe the database volume)")]
dev-down:
    docker compose down

[doc("Seed a local demo account (demo@mica.dev / password123) + workspace")]
seed-dev:
    docker exec -i mica-postgres psql -U mica -d mica < seeds/dev_seed.sql

[doc("Rebuild the pure-Rust WebAssembly bundle into deploy/web")]
dev-web:
    bash scripts/build-web-rust.sh

[doc("Launch the pure-Rust native desktop client (clients/mica_rust)")]
app:
    cargo run --manifest-path clients/mica_rust/Cargo.toml --bin mica-rust

# ---------------------------------------------------------------- tests & gates

[doc("Run all Rust tests (root workspace + gated SQLite store + clients/mica_rust)")]
test:
    db=$(source scripts/lib-local.sh && resolve_database_url); \
    if [ -n "$db" ]; then \
        echo "==> Postgres-backed tests: ON ($db)"; \
        DATABASE_URL=$db cargo test --workspace; \
    else \
        echo "==> WARNING: no reachable Postgres, so Postgres-backed tests will SKIP"; \
        cargo test --workspace; \
    fi
    cargo test -p mica-core --features store
    cargo test --manifest-path clients/mica_rust/Cargo.toml

[doc("Run the CI gates locally (part: all | rust | client)")]
ci part="all":
    bash scripts/local-ci.sh {{part}}

[doc("Static analysis across root workspace and pure-Rust client")]
check:
    cargo clippy --workspace --all-targets -- -D warnings
    cargo clippy --manifest-path clients/mica_rust/Cargo.toml --all-targets -- -D warnings

[doc("Per-file Rust line-count gate (absolute + ratchet + delta; same as CI)")]
check-structure:
    bash scripts/check-file-size.sh

[doc("Rust line-coverage summary (cargo-llvm-cov)")]
coverage package="":
    cargo llvm-cov {{ if package == "" { "--workspace" } else { "-p " + package } }} --summary-only

[doc("SDK readiness: mica-md crate + npm package (no publish)")]
sdk-check:
    bash scripts/sdk-check.sh

# ------------------------------------------------------- local artifacts

[doc("1/4  mica-cli binary for this host -> target/release/")]
build-cli:
    cargo build --locked -p mica-cli --release
    @echo "-> target/release/mica-cli"

[doc("2/4  Pure-Rust Windows desktop installer -> clients/mica_rust/installer/Output/ (needs Inno Setup)")]
build-installer version:
    cargo build --manifest-path clients/mica_rust/Cargo.toml --bin mica-rust --release
    "/c/Program Files (x86)/Inno Setup 6/ISCC.exe" //DAppVersion={{version}} 'clients\mica_rust\installer\mica.iss'
    @echo "-> clients/mica_rust/installer/Output/Mica-Setup-{{version}}.exe"

[doc("3/4  Pure-Rust WebAssembly bundle -> deploy/web (staged for the web image)")]
build-web:
    bash scripts/build-web-rust.sh

[doc("4/4  api-server binary for this host -> target/release/")]
build-api:
    cargo build --release -p mica-api-server
    @echo "-> target/release/mica-api-server"

[doc("Build cli + web + api")]
build-all: build-cli build-web build-api

# ---------------------------------------------------------------- docker

[doc("Build the prod images locally (CI-outage fallback)")]
docker-build tag: build-web
    docker build --provenance=false --sbom=false -f deploy/Dockerfile.api -t {{hub_acr}}/mica-api:{{tag}} .
    docker build --provenance=false --sbom=false -f deploy/Dockerfile.web -t {{hub_acr}}/mica-web:{{tag}} .
    docker build --provenance=false --sbom=false -f deploy/Dockerfile.cli -t {{hub_acr}}/mica-cli:{{tag}} .

[doc("Push locally built images to ACR (CI-outage fallback)")]
docker-push tag:
    docker push {{hub_acr}}/mica-api:{{tag}}
    docker push {{hub_acr}}/mica-web:{{tag}}
    docker push {{hub_acr}}/mica-cli:{{tag}}

# ---------------------------------------------------------------- deploy

[doc("Roll prod to a published version, e.g. `just deploy-prod 0.14.12` (+ --check --diff to rehearse)")]
deploy-prod version *flags:
    bash scripts/deploy-prod.sh "{{version}}" {{flags}}

[doc("Prove prod really serves <version>")]
verify-prod version:
    SITE='{{site}}' bash scripts/verify-prod.sh "{{version}}"

[doc("Prove a restore point really restores")]
restore-drill dump:
    NODE='{{node}}' bash scripts/restore-drill.sh "{{dump}}"

[doc("List the restore points on the node, newest last")]
restore-points:
    ssh {{node}} "ls -lah {{node_dir}}/*.sql.gz 2>/dev/null | tail -20 || echo '(none)'"

# ── Release gate & train ─────────────────────────────────────────────────────

[doc("Bump + gate + commit + tag, e.g. `just release 0.14.12`. Does not push.")]
release version:
    bash scripts/release.sh "{{version}}"

[doc("Everything that must be true before tagging. Refuses; does not warn.")]
release-check:
    bash scripts/release-check.sh

[doc("Just the version numbers + Cargo.lock. `just release` is the one you want.")]
release-bump version:
    bash scripts/release-bump.sh "{{version}}"

[doc("Everything on this machine: preflight → release → push → artifacts → images → upload → publish")]
ship version:
    bash scripts/ship.sh "{{version}}"

[doc("What the local release train needs, all reported at once")]
release-preflight version:
    bash scripts/release-preflight.sh "{{version}}"

[doc("Build the release assets THIS host can build into dist/vX.Y.Z/")]
release-artifacts version *flags:
    bash scripts/release-artifacts.sh "{{version}}" {{flags}}

[doc("Linux .deb into dist/vX.Y.Z/")]
build-deb version:
    bash scripts/build-linux-deb.sh "{{version}}" "dist/v{{version}}"

[doc("Attach dist/vX.Y.Z/* to the (draft) GitHub release vX.Y.Z")]
release-upload version:
    bash scripts/release-upload.sh "{{version}}"

[doc("Publish the draft release once every required asset is attached")]
release-publish version:
    bash scripts/release-publish.sh "{{version}}"

[doc("Build + push mica-api/web/cli images to ACR and Docker Hub (PUSH=0 to only build)")]
release-images version:
    bash scripts/release-images.sh "{{version}}"
