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
source scripts/lib-receipt.sh

part=${1:-all}
case "$part" in all | rust | flutter) ;; *) fail "unknown part '$part' (all | rust | flutter)" ;; esac

note "source files hidden by .gitignore"
bash scripts/check-ignored-sources.sh

note "SQL migrations: tracking and exact line endings"
bash scripts/check-migration-tracking.sh
bash scripts/test-migration-tracking.sh

note "CI configuration: job selection regressions"
bash scripts/test-ci-paths.sh
note "CI configuration: Flutter SDK reuse regressions"
bash scripts/test-flutter-sdk-reuse.sh

if [ "$part" != flutter ]; then
  note "structure: per-unit size gate"
  bash scripts/check-file-size.sh

  # rustfmt.toml was the law on paper only — nothing ran it, and 2/4-space
  # indentation mixed inside single files (2026-10-21 review 工程链). Same
  # command as ci.yml's rust job; the existing tree's one-time normalization
  # is a whitespace-only commit of its own (see #88). Root workspace only.
  note "rust: fmt (--check, as CI)"
  cargo fmt --all --check

  db=$(resolve_database_url)
  [ -n "$db" ] || fail "no reachable Postgres, so the DB-backed tests would SKIP and still report passed.
  Start one with 'just dev' (or export DATABASE_URL=postgres://… for your own)."
  export DATABASE_URL=$db
  # Belt to the braces above: every DB-backed test goes through
  # mica_infra::test_database_url(), which panics instead of skipping when this
  # is set — so a URL lost between here and the test process is a failure too.
  export MICA_REQUIRE_DB=1
  note "DB-backed tests: ON ($DATABASE_URL)"

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
fi

if [ "$part" != rust ]; then
  require_flutter_pin "CI tests the pin"
  bash scripts/build-md-wasm.sh
  (
    cd clients/mica_flutter
    "$FLUTTER" pub get --enforce-lockfile
    "$FLUTTER" gen-l10n
    note "flutter: analyze (--no-fatal-infos, as CI)"
    "$FLUTTER" analyze --no-fatal-infos
    # The FFI-backed unit tests load rust/target/release and verify its source
    # fingerprint (test/support/native_build.dart), so build it from this tree.
    note "flutter: FFI library for the unit tests"
    cargo build --release --manifest-path rust/Cargo.toml
    note "flutter: unit tests"
    flutter_receipt 1000 test
    note "flutter: web compiles (dart2js resolves the web stubs)"
    "$FLUTTER" build web --release --no-web-resources-cdn
    bash tool/gfonts/mirror.sh --check
  )
  # Browser-only suites skip or compile to a stub on the VM run above. The
  # Windows test server 404s its own CanvasKit, so there they stay manual.
  if [ "$(host_os)" = windows ]; then
    browser_note="    - browser-only Flutter suites ......... bash scripts/browser-tests.sh   (Linux/macOS)"
  else
    note "flutter: browser-only suites in Chrome"
    bash scripts/browser-tests.sh
  fi
fi

cat <<'EOF'

  ALL GREEN for what this ran. NOT covered here (each has its own command):
    - web e2e against a real stack ........ just dev && just web-e2e
    - Windows integration tests (FFI) ...... just test-integration   (Windows desktop only)
    - container boot of the api image ...... just docker-build <tag> && docker compose -f deploy/docker-compose.single.yml up
    - MCP inspector conformance ............ needs npx @modelcontextprotocol/inspector
EOF
[ -z "${browser_note:-}" ] || echo "$browser_note"
