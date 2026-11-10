#!/usr/bin/env bash
# Build the editor's opt-in renderers, separately from startup Markdown/CRDT.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
bash scripts/check-md-wasm-toolchain.sh

target_dir=$(cargo metadata --format-version 1 --no-deps --locked | jq -er .target_directory)
out=clients/mica_flutter/web/mica_preview
mkdir -p "$out"
for kind in math mermaid; do
  CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
    CARGO_PROFILE_RELEASE_PANIC=abort CARGO_PROFILE_RELEASE_OPT_LEVEL=z \
    cargo build --locked -p mica-md-wasm --lib --release --target wasm32-unknown-unknown \
      --no-default-features \
      --features "preview-$kind"
  wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section \
    --out-dir "$out" --out-name "$kind" "$target_dir/wasm32-unknown-unknown/release/mica_md_wasm.wasm"
  wasm=$(gzip -n -9c "$out/${kind}_bg.wasm" | wc -c)
  js=$(gzip -n -9c "$out/$kind.js" | wc -c)
  size=$((wasm + js))
  case "$kind" in
    math) budget=1048576 ;;
    mermaid) budget=2097152 ;;
  esac
  [ "$size" -le "$budget" ] || fail "$kind preview gzip is $size B, hard budget $budget B"
  note "$kind preview gzip -n -9: wasm=$wasm B JS=$js B total=$size B (hard budget $budget B)"
done

support=clients/mica_flutter/test/support
mkdir -p "$support/mica_preview"
cp "$out/"*.{js,wasm} "$support/mica_preview/"
cp clients/mica_flutter/web/mica_preview_worker.js "$support/"
