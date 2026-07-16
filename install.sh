#!/bin/sh
# Mica CLI installer for Linux and macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/weironz/mica/main/install.sh | sh
#
# Re-run any time to update to the latest release. To pin a version:
#   MICA_VERSION=0.5.4 curl -fsSL https://raw.githubusercontent.com/weironz/mica/main/install.sh | sh
#
# Installs `mica-cli` to ~/.local/bin (override with MICA_BIN_DIR). Downloads the
# prebuilt binary from the GitHub release — nothing is built.
set -eu

repo="weironz/mica"
os="$(uname -s)"
arch="$(uname -m)"

# Only the platforms the release CI actually builds are supported.
case "$os" in
  Linux)
    case "$arch" in
      x86_64 | amd64) suffix="linux-x64" ;;
      *) echo "no prebuilt mica-cli for linux/$arch — build from source: cargo build --release -p mica-cli" >&2; exit 1 ;;
    esac ;;
  Darwin)
    case "$arch" in
      arm64 | aarch64) suffix="macos-arm64" ;;
      *) echo "prebuilt mica-cli is Apple Silicon only (got macOS/$arch) — build from source: cargo build --release -p mica-cli" >&2; exit 1 ;;
    esac ;;
  *) echo "unsupported OS: $os" >&2; exit 1 ;;
esac

version="${MICA_VERSION:-}"
version="${version#v}"
if [ -z "$version" ]; then
  # Pull just the tag_name out of the latest-release JSON — no jq dependency.
  tag="$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" \
    | grep '"tag_name"' | head -1 | sed -E 's/.*"tag_name" *: *"([^"]+)".*/\1/')"
  [ -n "$tag" ] || { echo "could not resolve the latest release" >&2; exit 1; }
  version="${tag#v}"
fi

asset="mica-cli-${version}-${suffix}"
url="https://github.com/$repo/releases/download/v${version}/${asset}"
dir="${MICA_BIN_DIR:-$HOME/.local/bin}"
dest="$dir/mica-cli"

echo "Installing mica-cli ${version} -> ${dest}"
mkdir -p "$dir"
curl -fSL "$url" -o "$dest"
chmod +x "$dest"

case ":$PATH:" in
  *":$dir:"*) : ;;
  *) echo "Note: $dir is not on your PATH. Add it, e.g.:  export PATH=\"$dir:\$PATH\"" ;;
esac

"$dest" --version || true
echo ""
echo "Installed. Next:"
echo "  mica-cli auth login --server https://mica.cloudcele.com --email you@example.com"
echo "Re-run this line any time to update."
