#!/usr/bin/env bash
# Package a built Flutter Linux bundle as a .deb, for manual install:
#   sudo apt install ./Mica-X.Y.Z-linux-amd64.deb
#
#   bash scripts/package-deb.sh <version> <bundle-dir> <out-dir>
#
# Layout: the bundle in /opt/mica, a `mica` command in /usr/bin, a desktop
# entry and an icon. Depends is computed by dpkg-shlibdeps from what the
# binaries really link, so it is only as portable as the build host: release
# builds come from the Ubuntu 22.04 builder (scripts/build-linux-deb.sh), which
# yields glibc >= 2.35 and the pre-"t64" package names that newer distros
# still provide. Needs dpkg-deb (and dpkg-shlibdeps for exact Depends).
set -euo pipefail
version=${1:?version}; bundle=$(realpath "${2:?bundle dir}"); out=$(realpath -m "${3:?out dir}")
cd "$(dirname "$0")/.."
[ -x "$bundle/mica_flutter" ] || { echo "no Linux bundle at $bundle" >&2; exit 1; }
command -v dpkg-deb >/dev/null || { echo "dpkg-deb not found (Debian/Ubuntu, or use the Docker builder)" >&2; exit 1; }

id=io.github.weironz.Mica
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir -p "$stage/DEBIAN" "$stage/opt/mica" "$stage/usr/bin" \
  "$stage/usr/share/applications" "$stage/usr/share/icons/hicolor/512x512/apps"
cp -a "$bundle/." "$stage/opt/mica/"
ln -s /opt/mica/mica_flutter "$stage/usr/bin/mica"
cp clients/mica_flutter/linux/packaging/$id.desktop "$stage/usr/share/applications/"
cp clients/mica_flutter/assets/logo_mark.png "$stage/usr/share/icons/hicolor/512x512/apps/$id.png"

depends="libgtk-3-0, libayatana-appindicator3-1, libstdc++6"
if command -v dpkg-shlibdeps >/dev/null; then
  # dpkg-shlibdeps wants a source tree; give it a throwaway one. -l: the
  # bundle's own libraries are resolved there and never become package deps.
  probe=$(mktemp -d)
  mkdir -p "$probe/debian"
  printf 'Source: mica\n\nPackage: mica\nArchitecture: amd64\n' > "$probe/debian/control"
  depends=$(cd "$probe" && dpkg-shlibdeps -O -l"$stage/opt/mica/lib" \
    "$stage/opt/mica/mica_flutter" "$stage"/opt/mica/lib/*.so 2>/dev/null \
    | sed -n 's/^shlibs:Depends=//p')
  rm -rf "$probe"
fi
# The GL stack is loaded with dlopen (libepoxy -> libEGL), so shlibdeps never
# sees it; a clean system without it aborts at start ("Couldn't open
# libEGL.so.1"). Mesa supplies the drivers, software ones included.
depends="$depends, libegl1, libgles2"
size=$(du -sk "$stage" | cut -f1)
cat > "$stage/DEBIAN/control" <<CONTROL
Package: mica
Version: $version
Section: editors
Priority: optional
Architecture: amd64
Installed-Size: $size
Depends: $depends
Recommends: libegl-mesa0, libgl1-mesa-dri
Maintainer: weironz <weironz@users.noreply.github.com>
Homepage: https://github.com/weironz/mica
Description: Mica - collaborative Markdown workspace
 Desktop client for Mica: local workspaces on this device and cloud
 workspaces on a Mica server, with real-time collaboration.
CONTROL
mkdir -p "$out"
deb="$out/Mica-$version-linux-amd64.deb"
dpkg-deb --root-owner-group -Zxz --build "$stage" "$deb" >/dev/null
echo "==> $deb"
