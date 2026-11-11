#!/usr/bin/env bash
# Scan both independently resolved Rust graphs; fresh advisories fail the gate.
# Install the qualified tool with: cargo install cargo-audit --locked --version 0.23.0
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
command -v cargo-audit >/dev/null || fail "cargo-audit is required: cargo install cargo-audit --locked --version 0.23.0"
for lock in Cargo.lock clients/mica_flutter/rust/Cargo.lock; do
  note "RustSec audit: $lock (vulnerabilities and warnings are errors)"
  cargo audit --file "$lock" --deny warnings "$@"
done
