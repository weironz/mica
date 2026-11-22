#!/usr/bin/env bash
# Pre-tag release gate (100% pure-Rust fullstack workspace + clients/mica_rust).
set -euo pipefail

source "$(dirname "$0")/lib-local.sh"
source "$(dirname "$0")/lib-receipt.sh"

export CARGO_PROFILE_DEV_DEBUG=${CARGO_PROFILE_DEV_DEBUG:-0}
export CARGO_PROFILE_TEST_DEBUG=${CARGO_PROFILE_TEST_DEBUG:-0}

bash scripts/check-ignored-sources.sh
bash scripts/check-file-size.sh
bash scripts/release-tree-check.sh
bash scripts/test-migration-tracking.sh

db=$(resolve_database_url)
[ -n "$db" ] \
  || fail "no reachable Postgres, so the DB-backed tests would SKIP and still report passed. Run 'just dev' first (or export DATABASE_URL)."
echo "==> DB-backed tests: ON (reachable test database)"

version=$(bash scripts/manifest-version.sh)
echo "==> version: $version (root + pure-Rust client agree)"

want_rust=$(sed -n 's/^channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)
have_rust=$(rustc --version | awk '{print $2}')
[ "$have_rust" = "$want_rust" ] || fail \
  "local rustc is $have_rust, rust-toolchain.toml pins $want_rust"
echo "==> Rust: $have_rust (matches rust-toolchain.toml)"
bash scripts/check-md-wasm-toolchain.sh

export DATABASE_URL=$db MICA_REQUIRE_DB=1
cargo_receipt 500 cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo_receipt 1 cargo test -p mica-core --features store

echo "==> pure-Rust client (clients/mica_rust): tests, clippy, native build"
cargo_receipt 15 cargo test --manifest-path clients/mica_rust/Cargo.toml
cargo clippy --manifest-path clients/mica_rust/Cargo.toml --all-targets -- -D warnings
cargo build --manifest-path clients/mica_rust/Cargo.toml --bin mica-rust --release

printf '\n  release-check passed for %s\n' "$version"
