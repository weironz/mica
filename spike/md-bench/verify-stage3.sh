#!/usr/bin/env bash
# Run: bash spike/md-bench/verify-stage3.sh > spike/md-bench/receipt-stage3.txt 2>&1
set -euo pipefail
cd "$(dirname "$0")/../.."
# shellcheck disable=SC1091
source scripts/lib-local.sh
# shellcheck disable=SC1091
source scripts/lib-receipt.sh
cargo_receipt 6 cargo test --manifest-path spike/md-wasm-p0/Cargo.toml --release --features uuid
shellcheck spike/md-bench/verify-stage3.sh
just check-structure
bash scripts/check-ignored-sources.sh
