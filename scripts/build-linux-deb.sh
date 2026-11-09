#!/usr/bin/env bash
# Build the Linux desktop .deb: Mica-X.Y.Z-linux-amd64.deb into <out-dir>.
#
#   bash scripts/build-linux-deb.sh 0.14.4 dist/v0.14.4      # Docker if running, else native
#   MICA_CLIENT_BUILD=native bash scripts/build-linux-deb.sh ...
#
# Opt-in release asset (`just release-artifacts X.Y.Z --deb`, or the release
# workflow's build_linux_deb input / MICA_BUILD_LINUX_DEB variable). Docker
# works the same from Windows (Docker Desktop + Git Bash) and Linux; native
# needs Linux with Flutter, Rust, the GTK dev packages and dpkg-dev.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
source scripts/lib-client-build.sh
version=${1:?usage: build-linux-deb.sh <version> <out-dir>}; out=${2:?out dir}
native_ok=0
[ "$(host_os)" = linux ] && command -v dpkg-deb >/dev/null && pkg-config --exists gtk+-3.0 2>/dev/null && native_ok=1
case "$(client_build_mode "$native_ok")" in
  docker) client_build_docker deb "$version" "$out" linux ;;
  native)
    [ "$native_ok" = 1 ] || fail "native .deb needs Linux with dpkg-deb and libgtk-3-dev (or use Docker)"
    FLUTTER="$FLUTTER" bash scripts/client-package-inner.sh deb "$version" "$out" ;;
esac
