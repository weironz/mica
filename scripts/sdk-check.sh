#!/usr/bin/env bash
# SDK readiness, one command: packaging, docs, examples, npm. Everything the
# release checklist needs EXCEPT the publish actions themselves (T5.12, and
# only after user approval).
#
#   bash scripts/sdk-check.sh
#
# `cargo publish --dry-run` is intentionally NOT here: it refuses while
# `publish = false` is set, and un-setting it just to dry-run risks shipping a
# half-decided state. The package list check below covers the same surface
# (what would be packed) without touching the publish flag.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
source scripts/lib-receipt.sh

note "==> SDK check: mica-md"

# 1. The packaged file list: no fixtures, no diagnostics, only the four
#    documented examples. Fail on any unexpected entry.
mapfile -t pkg < <(cargo package -p mica-md --allow-dirty --list | tr '\\' '/')
expected=(
  "CHANGELOG.md"
  "README.md"
  "SCHEMA.md"
  "THIRD_PARTY.md"
  "build.rs"
  "examples/inline.rs"
  "examples/parse_and_serialize.rs"
  "examples/render_html.rs"
  "examples/table_cells.rs"
)
# src/ files follow the crate; cargo itself adds .cargo_vcs_info.json when
# packaging from a git checkout. Anything else is a packaging surprise.
for f in "${pkg[@]}"; do
  case "$f" in
    # src/, the manifest + its cargo-generated companions are always fine.
    src/*|.cargo_vcs_info.json|Cargo.toml|Cargo.toml.orig|Cargo.lock) ;;
    *) grep -qx "$f" <(printf '%s\n' "${expected[@]}") ||
      fail "unexpected file in package: $f" ;;
  esac
done
note "package list: ${#pkg[@]} files (src + manifest + $(printf '%s\n' "${expected[@]}" | wc -l) docs/examples)"

# 2. Docs build clean with warnings denied, all features.
RUSTDOCFLAGS="-D warnings" cargo doc -p mica-md --no-deps --all-features
note "cargo doc: clean (-D warnings, --all-features)"

# 3. The API/docs/robustness gates (also run in cargo test, but here they are
#    the point: each must PASS for the SDK to be releasable).
cargo test -q -p mica-md --test public_api --test schema_doc_sync --test never_panic
note "public_api / schema_doc_sync / never_panic: passed"
# The wasm binding builds with no default features; the whole suite must
# compile and pass there too (public_api once named a uuid-only variant).
cargo test -q -p mica-md --no-default-features
note "mica-md --no-default-features: passed"

# 4. Every shipped example runs.
for e in parse_and_serialize render_html table_cells inline; do
  cargo run -q -p mica-md --example "$e" >/dev/null
  note "example $e: ran"
done

# 5. The npm package: build both wasm targets, run its tests, check the
#    packed file list.
bash scripts/build-md-npm.sh
node --test packages/mica-md-js/test/basic.test.mjs
npm pack --dry-run --pack-destination /tmp "file:packages/mica-md-js" >/dev/null
note "npm package: tests passed, pack dry-run ok"

note "SDK check: ALL GREEN"