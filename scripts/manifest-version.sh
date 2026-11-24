#!/usr/bin/env bash
# Print the version this working tree claims to be — or refuse if the root
# workspace and pure-Rust client manifests disagree.
set -euo pipefail

rust_root=$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
rust_client=$(grep -m1 '^version' clients/mica_rust/Cargo.toml | sed 's/.*"\(.*\)".*/\1/')

[ -n "$rust_root" ] && [ -n "$rust_client" ] || {
  echo "REFUSED: could not read a version (Cargo.toml='$rust_root' clients/mica_rust/Cargo.toml='$rust_client'). Run from the repo root." >&2
  exit 1
}

[ "$rust_root" = "$rust_client" ] || {
  echo "REFUSED: version numbers disagree: Cargo.toml=$rust_root clients/mica_rust/Cargo.toml=$rust_client" >&2
  exit 1
}

printf '%s\n' "$rust_root"
