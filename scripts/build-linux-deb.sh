#!/usr/bin/env bash
# Build the pure-Rust Linux desktop .deb: Mica-X.Y.Z-linux-amd64.deb into <out-dir>.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=${1:?usage: build-linux-deb.sh <version> <out-dir>}
out=${2:?usage: build-linux-deb.sh <version> <out-dir>}

[ "$(host_os)" = linux ] && command -v dpkg-deb >/dev/null \
  || fail "build-linux-deb.sh requires Linux with dpkg-deb"

cargo build --manifest-path clients/mica_rust/Cargo.toml --bin mica-rust --release --locked
mkdir -p "$out"
bash scripts/package-deb.sh "$version" clients/mica_rust/target/release/mica-rust "$out/Mica-$version-linux-amd64.deb"
