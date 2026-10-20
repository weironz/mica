#!/usr/bin/env bash
# The CI workflow (.github/workflows/ci.yml), on this machine.
#
#   just ci                # everything below
#   just ci rust           # only the Rust half
#   just ci flutter        # only the Flutter half
#
# For when GitHub Actions minutes are gone: the same gates, run the same way,
# REFUSING rather than warning where CI would fail. In particular the
# DB-backed tests: they skip without a database and a skipped test reports as
# passed, so a missing database is a refusal here, not a footnote.
#
# What this does NOT cover is printed at the end, every time — a gate that is
# silent about its own edges reads as a gate over everything.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

part=${1:-all}
case "$part" in all | rust | flutter) ;; *) fail "unknown part '$part' (all | rust | flutter)" ;; esac

if [ "$part" != flutter ]; then
  note "structure: per-file size gate"
  bash scripts/check-file-size.sh

  db=$(resolve_database_url)
  [ -n "$db" ] || fail "no reachable Postgres, so the DB-backed tests would SKIP and still report passed.
  Start one with 'just dev' (or export DATABASE_URL=postgres://… for your own)."
  export DATABASE_URL=$db
  note "DB-backed tests: ON ($DATABASE_URL)"

  note "rust: build"
  cargo build --workspace
  note "rust: clippy (-D warnings, as CI)"
  cargo clippy --workspace --all-targets -- -D warnings
  note "rust: migrations"
  cargo run -q -p mica-infra --example apply_migrations
  note "rust: tests"
  cargo test --workspace
  note "rust: mica-core gated SQLite store"
  cargo test -p mica-core --features store
fi

if [ "$part" != rust ]; then
  want=$(sed -n 's/.*"flutter"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' .fvmrc)
  have=$("$FLUTTER" --version 2>/dev/null | sed -n 's/^Flutter \([0-9][^ ]*\).*/\1/p' | head -1)
  [ "$have" = "$want" ] || fail "local Flutter is '$have', the pin is $want (.fvmrc) — CI tests the pin"
  note "flutter $have (matches .fvmrc)"
  (
    cd clients/mica_flutter
    "$FLUTTER" pub get
    note "flutter: analyze (--no-fatal-infos, as CI)"
    "$FLUTTER" analyze --no-fatal-infos
    note "flutter: unit tests"
    "$FLUTTER" test test
    note "flutter: web compiles (dart2js resolves the web stubs)"
    "$FLUTTER" build web --release --no-web-resources-cdn
    bash tool/gfonts/mirror.sh --check
  )
fi

cat <<'EOF'

  ALL GREEN for what this ran. NOT covered here (each has its own command):
    - web e2e against a real stack ........ just dev && just web-e2e
    - Windows integration tests (FFI) ...... just test-integration   (Windows desktop only)
    - container boot of the api image ...... just docker-build <tag> && docker compose -f deploy/docker-compose.single.yml up
    - MCP inspector conformance ............ needs npx @modelcontextprotocol/inspector
EOF
