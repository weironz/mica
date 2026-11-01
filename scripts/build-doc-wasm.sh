#!/usr/bin/env bash
# Build the web client's CRDT replica: mica-core's document core (yrs) as wasm.
# Generated bundles are not committed (see .gitignore), same as mica_md.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
# Same pinned wasm-bindgen-cli as the Markdown engine (both bindings pin 0.2.129).
bash scripts/check-md-wasm-toolchain.sh

# yrs and uuid reach getrandom 0.3, whose browser backend is opted into by cfg.
# Scoped to the wasm target so host build scripts and proc-macros are untouched.
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
  CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_PANIC=abort \
  cargo build -p mica-doc-wasm --target wasm32-unknown-unknown --release --locked
target_dir=$(cargo metadata --format-version 1 --no-deps --locked | jq -er .target_directory)
out=clients/mica_flutter/web/mica_doc
# --weak-refs: a document object lives in wasm memory until freed. The Dart
# side frees scratch documents itself; for the long-lived replicas this lets
# the browser's FinalizationRegistry free the Rust side when the JS wrapper is
# collected, instead of leaking every document a tab ever opened.
# --no-modules-global: mica_md's no-modules glue already declares the global
# `wasm_bindgen`; a second declaration on the same page is a SyntaxError.
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target no-modules --no-modules-global mica_doc_bindgen --no-typescript --weak-refs --remove-name-section --remove-producers-section \
  --out-dir "$out" --out-name mica_doc "$target_dir/wasm32-unknown-unknown/release/mica_doc_wasm.wasm"

# The doc engine is fetched in parallel with mica_md and awaited at startup, so
# it has its own hard budget (measured 2026-11-01: ~244 KB). Exact bytes.
wasm_bytes=$(gzip -n -9 -c "$out/mica_doc_bg.wasm" | wc -c)
js_bytes=$(gzip -n -9 -c "$out/mica_doc.js" | wc -c)
total_bytes=$((wasm_bytes + js_bytes))
[ "$total_bytes" -le 262144 ] || fail "mica_doc gzip is $total_bytes B (wasm $wasm_bytes + JS $js_bytes), hard budget 262144 B"
cargo run --locked -q -p mica-doc-wasm --example print_fingerprint > "$out/SOURCE_FINGERPRINT"

support=clients/mica_flutter/test/support/mica_doc
rm -rf "$support"
mkdir -p "$support"
cp -r "$out/." "$support/"
note "mica_doc gzip -n -9: wasm=$wasm_bytes B JS=$js_bytes B total=$total_bytes B (hard budget 262144 B)"
note "mica_doc -> $out; browser support -> $support"
