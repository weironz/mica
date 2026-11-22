#!/usr/bin/env bash
# Everything docs/ops/release.md says to check before tagging — as a command that
# REFUSES, not a list somebody has to remember.
#
# The rule that forced this into existence: the Postgres-backed tests SKIP when
# DATABASE_URL is unset, and a skipped test reports as PASSED. So "I ran the
# tests" and "the tests ran" were different facts, and the only thing standing
# between them was a human remembering the difference. That is not a control.
#
# Lives in scripts/ rather than as a `just` shebang recipe on purpose: a shebang
# recipe is executed by `just` itself, which needs `cygpath` to translate the
# interpreter path on Windows — and cygpath is absent unless the caller already
# fixed their PATH. Recipes that call `bash scripts/…` run under the justfile's
# `windows-shell` (Git Bash) instead, where the whole toolchain is present.
set -euo pipefail

source "$(dirname "$0")/lib-local.sh"
source "$(dirname "$0")/lib-receipt.sh"

# No debug info in the gate's dev/test builds, as in local-ci.sh: full debug
# info for the workspace's test binaries is ~10 GB. Export to override.
export CARGO_PROFILE_DEV_DEBUG=${CARGO_PROFILE_DEV_DEBUG:-0}
export CARGO_PROFILE_TEST_DEBUG=${CARGO_PROFILE_TEST_DEBUG:-0}

bash scripts/check-ignored-sources.sh
bash scripts/release-tree-check.sh
bash scripts/test-migration-tracking.sh

# 1. A database must be UP — not "warn and continue" the way `just test` does.
#    At release time a silent skip IS the failure being prevented. The dev
#    stack's container, or any reachable $DATABASE_URL (a machine without the
#    dev stack — e.g. building releases locally when CI is unavailable).
db=$(resolve_database_url)
[ -n "$db" ] \
  || fail "no reachable Postgres, so the DB-backed tests would SKIP and still report passed. Run 'just dev' first (or export DATABASE_URL)."
echo "==> DB-backed tests: ON (reachable test database)"

# 2. The three version numbers must already agree. The reading lives in
#    scripts/manifest-version.sh because release.yml needs the same answer to
#    check the TAG against them, and two copies of three greps is the drift this
#    repo keeps paying for. It prints its own REFUSED and exits non-zero, which
#    `set -e` turns into ours.
version=$(bash scripts/manifest-version.sh)
echo "==> version: $version (three places agree)"

# 2b. The TOOLCHAINS must be the pinned ones, on this machine too.
#
#     Everything used to float on "stable": CI resolved one Flutter, the laptop
#     had another, and the shipped Docker image a third Rust — with no record of
#     any of it. That is how Flutter 3.47 turned Windows over to Impeller and
#     shipped a new renderer to users without anyone deciding, while this machine
#     stayed on 3.44 and debugged a different engine than the one it releases.
#
#     Checked HERE because CI cannot see a laptop. The pins themselves live in
#     .fvmrc (read by every flutter-action step) and rust-toolchain.toml (read by
#     rustup); this only asserts the machine agrees with them.
require_flutter_pin "CI builds what you are about to ship with the PIN, so testing on anything else tests a different engine. Run 'flutter upgrade', or check out the pin."

want_rust=$(sed -n 's/^channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' rust-toolchain.toml)
have_rust=$(rustc --version | awk '{print $2}')
[ "$have_rust" = "$want_rust" ] || fail \
  "local rustc is $have_rust, rust-toolchain.toml pins $want_rust — rustup honours that file automatically, so this usually means the pin was just changed and the toolchain is not installed: rustup toolchain install $want_rust"
echo "==> Rust: $have_rust (matches rust-toolchain.toml)"
bash scripts/check-md-wasm-toolchain.sh

# 3. Run the gates the SAME WAY CI runs them. `cargo check` does not run clippy,
#    and clippy without `-D warnings` exits 0 while printing the very thing that
#    fails CI — both have turned a pipeline red here before.
#    Receipts (scripts/lib-receipt.sh): a passed count, not just exit 0; and
#    MICA_REQUIRE_DB turns any DB test that would still skip into a panic.
export DATABASE_URL=$db MICA_REQUIRE_DB=1
cargo_receipt 500 cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
#    The Flutter half is spelled EXACTLY as ci.yml spells it, flags included.
#    It was `analyze lib` with no flags, which is stricter than CI — and with
#    ~130 long-standing `info` lints in the package, `flutter analyze` exits 1
#    and the gate could NEVER pass. A gate that always refuses does not stop bad
#    releases; it gets bypassed, and then nothing is checked at all.
#
#    `--no-fatal-warnings` is NOT here, and that is the deliberate half.
#    Warnings are fatal in both this gate and CI as of 2026-08-25, because an
#    `invalid_return_type_for_catch_error` warning sat unread long enough to
#    ship a real defect: a `.catchError` handler returning the wrong type, so
#    the error path — whose entire job was to fail quietly — threw a TypeError
#    instead. Two warnings existed when that was found; both are gone, and the
#    bar is met today, which is the only condition under which raising it is
#    honest. The one genuine false positive is suppressed at its own line
#    (`// ignore:` in main.dart, where the analyzer and the CFE disagree).
#
#    Infos stay non-fatal: ~130 of them, long-standing, style-level. Making them
#    fatal would be a repo-wide sweep nobody asked for.
#    The FFI-backed unit tests verify rust/target/release against this tree's
#    sources (test/support/native_build.dart), so build it first.
# Web freshness tests must compare a current bundle rather than skip for absence.
bash scripts/build-md-wasm.sh
bash scripts/build-doc-wasm.sh
bash scripts/build-preview-wasm.sh
bash scripts/build-board-wasm.sh
(cd clients/mica_flutter \
  && "${FLUTTER:-flutter}" pub get --enforce-lockfile \
  && "${FLUTTER:-flutter}" gen-l10n \
  && "${FLUTTER:-flutter}" analyze --no-fatal-infos \
  && cargo build --release --manifest-path rust/Cargo.toml \
  && flutter_receipt 5 --tags perf --concurrency=1 \
    test/markdown_codec_bench_test.dart test/perf/html_paste_bench_test.dart test/perf/table_cells_bench_test.dart \
  && flutter_receipt 1000 --exclude-tags perf test)

# ── The desktop FFI crate is not in the root workspace ───────────────────────
# `clients/mica_flutter/rust` (rust_lib_mica_flutter) is its own cargo project,
# so `cargo test --workspace` above never compiles it. Change a shared crate —
# mica-interchange, mica-markdown, mica-core — and every local check can pass
# while the desktop build is broken.
#
# Found 2026-08-30 the hard way: a field added to `mica_interchange::TreeNode`
# was wired through the server's caller and not the local store's. 1481 Flutter
# tests and the whole cargo workspace went green; `flutter run -d windows` died
# on E0063. CI does catch it (flutter-integration.yml builds the crate via
# cargokit) — but only AFTER the tag is pushed, which is the wrong side of the
# gate. Seconds when warm, and it turns a failed release train into a refusal.
(cd clients/mica_flutter/rust && cargo check --quiet)   || fail "clients/mica_flutter/rust does not compile — it is NOT part of the root workspace, so nothing above would have told you. Fix it before tagging; CI would only find this after the push."
echo "==> desktop FFI crate compiles"

# Flutter's Cargokit used to force floating stable even when direct cargo used
# rust-toolchain.toml. Check the actual prepare/build command selection too.
(cd clients/mica_flutter/rust_builder/cargokit/build_tool \
  && dart pub get \
  && dart run test/toolchain_check.dart)
echo "==> Cargokit respects the repository Rust toolchain"

printf '\n  release-check passed for %s\n' "$version"
