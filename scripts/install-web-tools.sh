#!/usr/bin/env bash
# Install pinned wasm-bindgen-cli (matching Cargo.lock), binaryen wasm-opt, and
# brotli on Linux CI runners into $HOME/.local (cacheable via actions/cache)
# before running scripts/build-web-rust.sh.
set -euo pipefail
cd "$(dirname "$0")/.."

WBG_VER=$(awk '/^name = "wasm-bindgen"$/ { found=1; next } found && /^version = / { gsub(/"/, "", $3); print $3; found=0; exit }' clients/mica_rust/Cargo.lock)
BINARYEN_VER="${MICA_BINARYEN_VER:-123}"
PREFIX="${MICA_WEB_TOOLS_PREFIX:-$HOME/.local}"

mkdir -p "$PREFIX/bin" "$PREFIX/lib"
export PATH="$PREFIX/bin:$PATH"
if [[ -n "${GITHUB_PATH:-}" ]]; then
  echo "$PREFIX/bin" >> "$GITHUB_PATH"
fi

if [[ "$(wasm-bindgen --version 2>/dev/null || true)" != "wasm-bindgen ${WBG_VER}" ]]; then
  echo "==> Installing wasm-bindgen ${WBG_VER} into ${PREFIX}/bin..."
  curl -fsSL "https://github.com/rustwasm/wasm-bindgen/releases/download/${WBG_VER}/wasm-bindgen-${WBG_VER}-x86_64-unknown-linux-musl.tar.gz" \
    | tar -xz --strip-components=1 -C "$PREFIX/bin" "wasm-bindgen-${WBG_VER}-x86_64-unknown-linux-musl/wasm-bindgen"
fi

if ! command -v wasm-opt >/dev/null 2>&1; then
  echo "==> Installing binaryen version_${BINARYEN_VER} (wasm-opt) into ${PREFIX}..."
  curl -fsSL "https://github.com/WebAssembly/binaryen/releases/download/version_${BINARYEN_VER}/binaryen-version_${BINARYEN_VER}-x86_64-linux.tar.gz" \
    | tar -xz --strip-components=1 -C "$PREFIX" "binaryen-version_${BINARYEN_VER}/bin/wasm-opt" "binaryen-version_${BINARYEN_VER}/lib"
fi

wasm-bindgen --version
wasm-opt --version
brotli --version
