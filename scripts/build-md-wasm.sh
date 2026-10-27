#!/usr/bin/env bash
# Build the ten production Markdown exports; generated bundles are not committed.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
bash scripts/check-md-wasm-toolchain.sh

CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_PANIC=abort cargo build -p mica-md-wasm --target wasm32-unknown-unknown --release --locked
# Respect Cargo's target directory override (CI and local builds may use one).
target_dir=$(cargo metadata --format-version 1 --no-deps --locked | jq -er .target_directory)
out=clients/mica_flutter/web/mica_md
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target no-modules --no-typescript --remove-name-section --remove-producers-section --out-dir "$out" --out-name mica_md \
  "$target_dir/wasm32-unknown-unknown/release/mica_md_wasm.wasm"

# Real HTTP transfers compress each file separately. -n removes timestamps and
# original names; compare exact bytes, never truncate to KiB before the gate.
wasm_bytes=$(gzip -n -9 -c "$out/mica_md_bg.wasm" | wc -c)
js_bytes=$(gzip -n -9 -c "$out/mica_md.js" | wc -c)
total_bytes=$((wasm_bytes + js_bytes))
[ "$total_bytes" -le 204800 ] || fail "mica_md gzip is $total_bytes B (wasm $wasm_bytes + JS $js_bytes), hard budget 204800 B"
cargo run --locked -q -p mica-md-wasm --example print_fingerprint > "$out/SOURCE_FINGERPRINT"

support=clients/mica_flutter/test/support/mica_md
rm -rf "$support"
mkdir -p "$support"
cp -r "$out/." "$support/"
# M2 requires the checked-in Rust wire gold corpus, never a test-local mirror.
gold=crates/md-core/tests/fixtures/wire
[ -d "$gold" ] || fail "wire gold corpus missing ($gold); M2 contracts are pending, not passed"
shopt -s nullglob
gold_files=("$gold"/*.json)
[ "${#gold_files[@]}" -gt 0 ] || fail "wire gold corpus is empty ($gold); regenerate the checked-in gold before building"
mkdir -p "$support/gold"
cp -r "$gold/." "$support/gold/"
note "mica_md gzip -n -9: wasm=$wasm_bytes B JS=$js_bytes B total=$total_bytes B (hard budget 204800 B)"
note "mica_md -> $out; browser support -> $support"
