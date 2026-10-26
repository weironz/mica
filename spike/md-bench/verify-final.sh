#!/usr/bin/env bash
# Count actual host assertions; never treat zero matching tests as success.
# Run: bash spike/md-bench/verify-final.sh > spike/md-bench/receipt-final.txt 2>&1
set -euo pipefail
cd "$(dirname "$0")/../.."
# shellcheck disable=SC1091
source scripts/lib-local.sh
# shellcheck disable=SC1091
source scripts/lib-receipt.sh
cargo_receipt 4 cargo test --manifest-path spike/md-wasm-p0/Cargo.toml --release --features uuid
shellcheck spike/md-bench/build_variants.sh spike/md-bench/verify-final.sh
just check-structure
bash scripts/check-ignored-sources.sh
