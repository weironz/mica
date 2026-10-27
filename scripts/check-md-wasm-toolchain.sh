#!/usr/bin/env bash
# wasm-bindgen's crate and CLI exchange a versioned schema; require exact pins.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

want=$(awk '/^name = "wasm-bindgen"$/ { found=1; next } found && /^version = / { gsub(/"/, "", $3); print $3; found=0 }' Cargo.lock)
[[ "$want" =~ ^0\.2\.[0-9]+$ ]] || fail "Cargo.lock must pin exactly one wasm-bindgen 0.2.x version (got '$want')"
have=$(wasm-bindgen --version 2>/dev/null || true)
[ "$have" = "wasm-bindgen $want" ] || fail "wasm-bindgen-cli is '${have:-missing}', Cargo.lock pins $want — cargo install wasm-bindgen-cli --version $want --locked"
note "$have (matches Cargo.lock)"
