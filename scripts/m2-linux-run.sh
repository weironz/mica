#!/usr/bin/env bash
# One-shot Linux recheck: install the pinned CLI, then run the formal build.
set -euo pipefail
rustup target add wasm32-unknown-unknown
cargo install --locked wasm-bindgen-cli --version 0.2.129 2>&1 | tail -1
wasm-bindgen --version
rustc --version
bash /src/m2-linux-build.sh
