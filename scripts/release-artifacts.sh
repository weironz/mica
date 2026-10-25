#!/usr/bin/env bash
# Build every GitHub-release asset THIS machine can build, into dist/vX.Y.Z/.
#
#   just release-artifacts 0.13.57             # what this host can build
#   just release-artifacts 0.13.57 --no-smoke  # skip the installer launch test
#
# The local twin of the `windows` + `cli` jobs in .github/workflows/release.yml:
#
#   asset                                  built on
#   Mica-Setup-X.Y.Z.exe                   Windows (Flutter + Inno Setup), smoke-tested
#   mica-cli-X.Y.Z-windows-x64.exe         Windows
#   mica-cli-X.Y.Z-linux-x64  (static)     Linux natively; Windows/macOS through Docker
#   mica-cli-X.Y.Z-macos-arm64             macOS
#
# So one Windows box with Docker Desktop produces everything except the macOS
# CLI. Run it on each machine you have, then `just release-upload X.Y.Z` from
# each; `just release-publish X.Y.Z` flips the release public once the
# required assets are all attached.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

version=${1:-}
check_version_arg "$version"
smoke=1
for a in "${@:2}"; do
  case "$a" in --no-smoke) smoke=0 ;; *) fail "unknown flag $a" ;; esac
done

require_tagged_tree "$version"
out=$(dist_dir "$version")
mkdir -p "$out"
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

# Linux CLI inside the SAME pinned toolchain image the api is built with, so a
# Windows or macOS machine can produce it too. Its own target dir, so it never
# fights the host build over target/.
build_cli_linux_docker() {
  note "mica-cli (linux-x64, static, in rust:$rust_pin-slim-bookworm)"
  local repo; repo=$(pwd)
  case "$repo" in /[a-z]/*) repo="$(echo "${repo:1:1}" | tr a-z A-Z):${repo:2}" ;; esac
  # RUSTUP_TOOLCHAIN overrides rust-toolchain.toml, whose `components`
  # (clippy, rustfmt) the slim image lacks — rustup would otherwise download
  # them for a build that uses neither. Same pinned version either way.
  MSYS_NO_PATHCONV=1 docker run --rm -v "$repo:/src" -w /src \
    -e CARGO_TARGET_DIR=/src/target/docker-musl -e "RUSTUP_TOOLCHAIN=$rust_pin" \
    "rust:$rust_pin-slim-bookworm" bash -ec '
      apt-get update -qq && apt-get install -y -qq musl-tools >/dev/null
      rustup target add x86_64-unknown-linux-musl >/dev/null
      cargo build --locked -p mica-cli --release --target x86_64-unknown-linux-musl'
  cp target/docker-musl/x86_64-unknown-linux-musl/release/mica-cli "$out/mica-cli-$version-linux-x64"
}

verify_static() {
  local f=$1 kind
  command -v file >/dev/null 2>&1 || { note "('file' not installed — static-link check skipped)"; return; }
  kind=$(file "$f")
  case "$kind" in
    *"statically linked"* | *"static-pie linked"*) note "static: ok" ;;
    *) fail "$f is not statically linked — it will refuse to start on older distros: $kind" ;;
  esac
}

case "$os" in
  windows)
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-installer-wait.ps1
    note "Windows installer"
    (cd clients/mica_flutter && "$FLUTTER" build windows --release)
    iscc=${ISCC:-"/c/Program Files (x86)/Inno Setup 6/ISCC.exe"}
    if [ -z "${ISCC:-}" ] && [ ! -x "$iscc" ] && [ -n "${LOCALAPPDATA:-}" ]; then
      iscc="$(cygpath -u "$LOCALAPPDATA")/Programs/Inno Setup 6/ISCC.exe"
    fi
    [ -x "$iscc" ] || fail "Inno Setup 6 compiler not found at $iscc — install it or set ISCC"
    "$iscc" "//DAppVersion=$version" 'clients\mica_flutter\installer\mica.iss'
    setup="clients/mica_flutter/installer/Output/Mica-Setup-$version.exe"
    cp "$setup" "$out/"
    if [ "$smoke" = 1 ]; then
      note "installer smoke test (install + launch)"
      powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-installer.ps1 "$(cygpath -w "$out/Mica-Setup-$version.exe")"
    fi
    build_cli_native windows-x64.exe mica-cli.exe
    if docker info >/dev/null 2>&1; then
      build_cli_linux_docker
      verify_static "$out/mica-cli-$version-linux-x64"
    else
      note "Docker is not running: the Linux CLI is left for a Linux machine (or start Docker Desktop and re-run)"
    fi
    ;;
  linux)
    if command -v musl-gcc >/dev/null 2>&1; then
      build_cli_native linux-x64 mica-cli x86_64-unknown-linux-musl
    elif docker info >/dev/null 2>&1; then
      build_cli_linux_docker
    else
      fail "need musl-tools (apt install musl-tools) or Docker to build the static Linux CLI"
    fi
    verify_static "$out/mica-cli-$version-linux-x64"
    note "the Windows installer and Windows CLI need a Windows machine"
    ;;
  macos)
    build_cli_native macos-arm64 mica-cli
    ;;
  *) fail "unsupported host $(uname -s)" ;;
esac

(cd "$out" && sha256sum -- * 2>/dev/null | grep -v ' sha256-' > "sha256-$os.txt" || true)
note "built:"
ls -l "$out"
