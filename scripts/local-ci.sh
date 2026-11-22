#!/usr/bin/env bash
# Local CI workflow (100% pure-Rust fullstack: server/core workspace + clients/mica_rust).
#
#   just ci                # everything below
#   just ci rust           # root workspace (L1+L2+server+CLI+MCP)
#   just ci client         # pure-Rust L3 client (clients/mica_rust)
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
source scripts/lib-receipt.sh

export CARGO_PROFILE_DEV_DEBUG=${CARGO_PROFILE_DEV_DEBUG:-0}
export CARGO_PROFILE_TEST_DEBUG=${CARGO_PROFILE_TEST_DEBUG:-0}

part=${1:-all}
case "$part" in all | rust | client) ;; *) fail "unknown part '$part' (all | rust | client)" ;; esac

note "Rust dependency vulnerabilities and warnings"
bash scripts/check-rust-audit.sh

note "source files hidden by .gitignore"
bash scripts/check-ignored-sources.sh

note "SQL migrations: tracking and exact line endings"
bash scripts/check-migration-tracking.sh
bash scripts/test-migration-tracking.sh

note "structure: per-unit size gate"
bash scripts/check-file-size.sh

if [ "$part" = all ] || [ "$part" = rust ]; then
  note "rust: fmt (--check, as CI)"
  cargo fmt --all --check

  db=$(resolve_database_url)
  [ -n "$db" ] || fail "no reachable Postgres, so the DB-backed tests would SKIP. Start one with 'just dev' or export DATABASE_URL."
  export DATABASE_URL=$db
  export MICA_REQUIRE_DB=1
  note "DB-backed tests: ON (reachable test database)"

  note "rust: build"
  cargo build --workspace
  note "rust: clippy (-D warnings, as CI)"
  cargo clippy --workspace --all-targets -- -D warnings
  note "rust: migrations"
  cargo run -q -p mica-infra --example apply_migrations
  note "rust: tests"
  cargo_receipt 500 cargo test --workspace
  note "rust: mica-core gated SQLite store"
  cargo_receipt 1 cargo test -p mica-core --features store
  note "rust: mica-md shipped examples"
  for e in parse_and_serialize render_html table_cells inline; do
    cargo run -q -p mica-md --example "$e" >/dev/null
  done
fi

if [ "$part" = all ] || [ "$part" = client ]; then
  note "client: clients/mica_rust clippy (-D warnings)"
  cargo clippy --manifest-path clients/mica_rust/Cargo.toml --all-targets -- -D warnings
  note "client: clients/mica_rust unit + storage/sync + headless E2E tests"
  cargo_receipt 15 cargo test --manifest-path clients/mica_rust/Cargo.toml
  note "client: clients/mica_rust native binary build"
  cargo build --manifest-path clients/mica_rust/Cargo.toml --bin mica-rust
fi

cat <<'EOF'

  ALL GREEN (100% Fullstack Rust). Optional host/stack checks:
    - Web WASM bundle build ............... bash scripts/build-web-rust.sh
    - SDK readiness (mica-md + npm) ....... just sdk-check
    - container stack boot ................ just docker-build <tag>
EOF
