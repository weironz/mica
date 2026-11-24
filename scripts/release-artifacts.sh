#!/usr/bin/env bash
# Build every GitHub-release asset THIS machine can build, into dist/vX.Y.Z/.
#   Mica-Setup-X.Y.Z.exe                   Windows (pure-Rust mica-rust.exe + Inno Setup)
#   mica-cli-X.Y.Z-windows-x64.exe         Windows
#   mica-cli-X.Y.Z-linux-x64  (static)     Linux natively; Windows/macOS via Docker
#   mica-cli-X.Y.Z-macos-arm64             macOS
#   Mica-X.Y.Z-linux-amd64.deb             --deb or MICA_BUILD_LINUX_DEB=1
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=${1:-}
check_version_arg "$version"
smoke=1
deb=${MICA_BUILD_LINUX_DEB:-0}
for a in "${@:2}"; do
  case "$a" in
    --no-smoke) smoke=0 ;;
    --deb) deb=1 ;;
    *) fail "unknown flag $a" ;;
  esac
done

require_tagged_tree "$version"
out=$(dist_dir "$version")
mkdir -p "$out"
rm -f "$out"/sha256-*.txt "$out/.built-from"
os=$(host_os)
note "host: $os  ->  $out/"

rust_pin=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml)

build_cli_native() {
  local suffix=$1 bin=$2 target=${3:-}
  note "mica-cli ($suffix)"
  if [ -n "$target" ]; then
    rustup target add "$target" >/dev/null
    cargo build --locked -p mica-cli --release --target "$target"
    cp "target/$target/release/$bin" "$out/mica-cli-$version-$suffix"
  else
    cargo build --locked -p mica-cli --release
    cp "target/release/$bin" "$out/mica-cli-$version-$suffix"
  fi
}

build_cli_linux_docker() {
  local builder="mica-cli-musl-builder:$rust_pin"
  if ! docker image inspect "$builder" >/dev/null 2>&1; then
    note "builder image $builder"
    printf '%s\n' \
      "FROM rust:$rust_pin-slim-bookworm" \
      "RUN apt-get update && apt-get install -y --no-install-recommends musl-tools \\" \
      " && rm -rf /var/lib/apt/lists/* && rustup target add x86_64-unknown-linux-musl" \
      | docker build -t "$builder" -
  fi
  note "mica-cli (linux-x64, static, in $builder)"
  local repo; repo=$(pwd)
  case "$repo" in /[a-z]/*) repo="$(echo "${repo:1:1}" | tr a-z A-Z):${repo:2}" ;; esac
  MSYS_NO_PATHCONV=1 docker run --rm -v "$repo:/src" -w /src \
    -e CARGO_TARGET_DIR=/src/target/docker-musl -e "RUSTUP_TOOLCHAIN=$rust_pin" \
    "$builder" cargo build --locked -p mica-cli --release --target x86_64-unknown-linux-musl
  cp target/docker-musl/x86_64-unknown-linux-musl/release/mica-cli "$out/mica-cli-$version-linux-x64"
}

verify_static() {
  local f=$1 kind
  command -v file >/dev/null 2>&1 || { note "('file' not installed — static-link check skipped)"; return; }
  kind=$(file "$f")
  case "$kind" in
    *"statically linked"* | *"static-pie linked"*) note "static: ok" ;;
    *) fail "$f is not statically linked: $kind" ;;
  esac
}

case "$os" in
  windows)
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-installer-wait.ps1
    note "Windows installer (pure-Rust mica-rust.exe)"
    cargo build --manifest-path clients/mica_rust/Cargo.toml --bin mica-rust --release --locked
    iscc=$(find_iscc) || fail "Inno Setup 6 compiler not found — install it or set ISCC"
    "$iscc" "//DAppVersion=$version" 'clients\mica_rust\installer\mica.iss'
    setup="clients/mica_rust/installer/Output/Mica-Setup-$version.exe"
    cp "$setup" "$out/"
    if [ "$smoke" = 1 ]; then
      note "installer smoke test (install + launch)"
      powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-installer.ps1 "$(cygpath -w "$out/Mica-Setup-$version.exe")" -ExpectedVersion "$version"
    fi
    build_cli_native windows-x64.exe mica-cli.exe
    if docker info >/dev/null 2>&1; then
      build_cli_linux_docker
      verify_static "$out/mica-cli-$version-linux-x64"
    else
      note "Docker is not running: the Linux CLI is left for a Linux machine"
    fi
    ;;
  linux)
    if command -v musl-gcc >/dev/null 2>&1; then
      build_cli_native linux-x64 mica-cli x86_64-unknown-linux-musl
    elif docker info >/dev/null 2>&1; then
      build_cli_linux_docker
    else
      fail "need musl-tools or Docker to build the static Linux CLI"
    fi
    verify_static "$out/mica-cli-$version-linux-x64"
    ;;
  macos)
    build_cli_native macos-arm64 mica-cli
    ;;
  *) fail "unsupported host $(uname -s)" ;;
esac

if [ "$deb" = 1 ]; then note "Linux .deb"; bash scripts/build-linux-deb.sh "$version" "$out"; fi

for manifest in sha256-windows.txt sha256-windows-installer.txt sha256-linux.txt sha256-macos.txt \
                sha256-linux-desktop.txt; do
  : > "$out/$manifest"
  for f in "$out"/*; do
    name=${f##*/}
    case "$name" in sha256-*.txt) continue ;; esac
    manifest_of_asset=$(manifest_for_asset "$name") \
      || fail "$name has no platform manifest"
    [ "$manifest_of_asset" = "$manifest" ] || continue
    hash=$(cd "$out" && sha256sum "$name" | cut -d' ' -f1)
    printf '%s  %s\n' "$hash" "$name" >> "$out/$manifest"
  done
  [ -s "$out/$manifest" ] || rm -f "$out/$manifest"
done

git rev-parse HEAD > "$out/.built-from"
note "built:"
ls -l "$out"
