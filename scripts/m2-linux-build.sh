#!/usr/bin/env bash
# Formal ten-API wasm build for the M2 Linux recheck (runs INSIDE the container).
set -euo pipefail
cd /src
export CARGO_PROFILE_RELEASE_LTO=true
export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
export CARGO_PROFILE_RELEASE_PANIC=abort
cargo build -p mica-md-wasm --target wasm32-unknown-unknown --release --locked
mkdir -p /out
wasm=$(find target -path '*wasm32-unknown-unknown/release/mica_md_wasm.wasm' | head -1)
[ -n "$wasm" ] || { echo "no mica_md_wasm.wasm under target" >&2; exit 1; }
wasm-bindgen --target no-modules --no-typescript --remove-name-section --remove-producers-section \
  --out-dir /out --out-name mica_md "$wasm"
cargo run --locked -q -p mica-md-wasm --example print_fingerprint > /out/SOURCE_FINGERPRINT
sha256sum /out/mica_md_bg.wasm /out/mica_md.js
gzip -n -9 -c /out/mica_md_bg.wasm | wc -c
gzip -n -9 -c /out/mica_md.js | wc -c
