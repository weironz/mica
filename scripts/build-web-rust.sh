#!/usr/bin/env bash
# Build Mica's pure-Rust WebAssembly client (`clients/mica_rust`) into deploy/web/.
# Single Rust stack (L1 + L2 + L3 NSH pure-Rust web runtime) — zero Flutter,
# zero Dart, zero CanvasKit, zero C/clang toolchain required.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> Building mica-rust for wasm32-unknown-unknown (--profile web)..."
cargo build \
  --manifest-path clients/mica_rust/Cargo.toml \
  --lib \
  --target wasm32-unknown-unknown \
  --profile web \
  --locked

target_dir=$(cargo metadata --manifest-path clients/mica_rust/Cargo.toml --format-version 1 --no-deps --locked | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])' 2>/dev/null || echo "clients/mica_rust/target")
wasm_path="$target_dir/wasm32-unknown-unknown/web/mica_rust.wasm"
if [[ ! -f "$wasm_path" ]]; then
  wasm_path="clients/mica_rust/target/wasm32-unknown-unknown/web/mica_rust.wasm"
fi
if [[ ! -f "$wasm_path" ]]; then
  echo "FAIL: expected $wasm_path not found" >&2
  exit 1
fi

# Locate nsh.js shipped inside the locked nsh-platform-web crate.
nsh_web_manifest=$(cargo metadata --manifest-path clients/mica_rust/Cargo.toml --format-version 1 --locked | python3 -c '
import json, sys
meta = json.load(sys.stdin)
for p in meta["packages"]:
    if p["name"] == "nsh-platform-web":
        print(p["manifest_path"])
        break
' 2>/dev/null || true)

nsh_js=""
if [[ -n "$nsh_web_manifest" ]]; then
  cand="$(dirname "$nsh_web_manifest")/web/nsh.js"
  [[ -f "$cand" ]] && nsh_js="$cand"
fi
if [[ -z "$nsh_js" ]]; then
  # Fallback without python3: search cargo registry directly
  cargo_home="${CARGO_HOME:-$HOME/.cargo}"
  nsh_js=$(find "$cargo_home/registry/src" -path "*/nsh-platform-web-0.6.1/web/nsh.js" 2>/dev/null | head -n 1 || true)
fi
if [[ -z "$nsh_js" || ! -f "$nsh_js" ]]; then
  echo "FAIL: could not locate nsh-platform-web web/nsh.js" >&2
  exit 1
fi

out="deploy/web"
rm -rf "$out"
mkdir -p "$out"
cp clients/mica_rust/web/index.html "$out/index.html"
cp "$nsh_js" "$out/nsh.js"
cp "$wasm_path" "$out/mica_rust.wasm"
chmod -R a+rX "$out"

raw_bytes=$(wc -c < "$out/mica_rust.wasm")
gz_bytes=$(gzip -n -9 -c "$out/mica_rust.wasm" | wc -c)
echo "==> deploy/web staged (pure-Rust Web client)"
echo "    mica_rust.wasm raw:    ${raw_bytes} B"
echo "    mica_rust.wasm gzip-9: ${gz_bytes} B"
