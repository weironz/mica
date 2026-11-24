#!/usr/bin/env bash
# Package the built pure-Rust Linux desktop binary as a .deb:
#   bash scripts/package-deb.sh <version> <mica-rust-binary> <out-deb>
set -euo pipefail
version=${1:?version}
binary=$(realpath "${2:?mica-rust binary}")
out=$(realpath -m "${3:?out deb}")
[ -x "$binary" ] || { echo "no Linux binary at $binary" >&2; exit 1; }
command -v dpkg-deb >/dev/null || { echo "dpkg-deb not found" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/DEBIAN" "$stage/usr/bin" "$stage/usr/share/applications"
cp "$binary" "$stage/usr/bin/mica"
chmod 0755 "$stage/usr/bin/mica"

cat > "$stage/usr/share/applications/io.github.weironz.Mica.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Mica
Comment=Collaborative Markdown & Whiteboard Workspace (Pure Rust)
Exec=/usr/bin/mica
Terminal=false
Categories=Office;Utility;
DESKTOP

size=$(du -sk "$stage" | cut -f1)
cat > "$stage/DEBIAN/control" <<CONTROL
Package: mica
Version: $version
Section: editors
Priority: optional
Architecture: amd64
Installed-Size: $size
Depends: libc6
Maintainer: weironz <weironz@users.noreply.github.com>
Homepage: https://github.com/weironz/mica
Description: Mica - collaborative Markdown & whiteboard workspace (Pure Rust)
 Fullstack Rust desktop client for Mica: local SQLite workspaces and cloud
 workspaces with real-time CRDT collaboration.
CONTROL

mkdir -p "$(dirname "$out")"
dpkg-deb --root-owner-group -Zxz --build "$stage" "$out" >/dev/null
echo "==> $out"
