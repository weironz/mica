#!/usr/bin/env bash
# Build the on-demand WebAssembly module for mica-board (0 B on normal pages).
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
bash scripts/check-md-wasm-toolchain.sh

CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
  CARGO_PROFILE_RELEASE_PANIC=abort CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
  cargo build -p mica-board-wasm --target wasm32-unknown-unknown --release

target_dir=$(cargo metadata --format-version 1 --no-deps | jq -er .target_directory)
out=clients/mica_flutter/web/mica_board
rm -rf "$out"
mkdir -p "$out"
wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
  --out-dir "$out" --out-name board "$target_dir/wasm32-unknown-unknown/release/mica_board_wasm.wasm"

wasm_bytes=$(gzip -n -9 -c "$out/board_bg.wasm" | wc -c)
js_bytes=$(gzip -n -9 -c "$out/board.js" | wc -c)
total_bytes=$((wasm_bytes + js_bytes))
# Hard budget: 256 KiB gzip-9 (on-demand only; never fetched on regular pages).
[ "$total_bytes" -le 262144 ] || fail "mica_board gzip is $total_bytes B (wasm $wasm_bytes + JS $js_bytes), hard budget 262144 B"
note "mica_board gzip -n -9: wasm=$wasm_bytes B JS=$js_bytes B total=$total_bytes B (hard budget 262144 B)"
