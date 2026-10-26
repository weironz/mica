#!/usr/bin/env bash
# Build every wasm variant for the P0 cost experiment and record gzip sizes.
# All use the repo-pinned stable Rust + wasm32-unknown-unknown; no nightly.
set -euo pipefail
cd "$(dirname "$0")/../.."
crate=spike/md-wasm-p0
out=spike/artifacts
wasm="$crate/target/wasm32-unknown-unknown/release/mica_md_wasm_p0.wasm"

bind() {
  rm -rf "$out/$1"
  mkdir -p "$out/$1"
  wasm-bindgen --target no-modules --no-typescript --out-dir "$out/$1" --out-name mica_md "$wasm"
  node -e '
    const fs=require("fs"),z=require("zlib"),p=process.argv[1]+"/";
    const w=fs.readFileSync(p+"mica_md_bg.wasm"),j=fs.readFileSync(p+"mica_md.js");
    console.log(JSON.stringify({variant:process.argv[1],
      wasm:w.length,js:j.length,
      wasmGzip:z.gzipSync(w,{level:9}).length,
      jsGzip:z.gzipSync(j,{level:9}).length,
      combinedGzip:z.gzipSync(Buffer.concat([w,j]),{level:9}).length}));
  ' "$out/$1"
}

CARGO_PROFILE_RELEASE_OPT_LEVEL=z cargo build --manifest-path "$crate/Cargo.toml" --target wasm32-unknown-unknown --release
bind opt-z

CARGO_PROFILE_RELEASE_OPT_LEVEL=s cargo build --manifest-path "$crate/Cargo.toml" --target wasm32-unknown-unknown --release
bind opt-s

CARGO_PROFILE_RELEASE_OPT_LEVEL=3 cargo build --manifest-path "$crate/Cargo.toml" --target wasm32-unknown-unknown --release
bind opt-3

cargo build --manifest-path "$crate/Cargo.toml" --target wasm32-unknown-unknown --release --features uuid
bind opt-z-uuid
