#!/usr/bin/env bash
# Build the Android APKs: Mica-X.Y.Z-android-{arm64,armv7}.apk into <out-dir>.
#
#   bash scripts/build-android-apk.sh 0.14.4 dist/v0.14.4    # Docker if running, else native
#   MICA_CLIENT_BUILD=native bash scripts/build-android-apk.sh ...
#
# Opt-in release asset (`just release-artifacts X.Y.Z --apk`, or the release
# workflow's build_android input / MICA_BUILD_ANDROID variable). Native works
# on Windows or Linux with an Android SDK (ANDROID_HOME, platform 36, NDK
# 28.2.13676358) and a JDK 17+. Signed with the build machine's debug key:
# for manual install, not a store (docs/ops/release.md「Android / Linux 安装包」).
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
source scripts/lib-client-build.sh
version=${1:?usage: build-android-apk.sh <version> <out-dir>}; out=${2:?out dir}
native_ok=0
[ -n "${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}" ] && native_ok=1
case "$(client_build_mode "$native_ok")" in
  docker) client_build_docker apk "$version" "$out" android ;;
  native)
    [ "$native_ok" = 1 ] || fail "native APK build needs ANDROID_HOME pointing at an Android SDK (or use Docker)"
    FLUTTER="$FLUTTER" bash scripts/client-package-inner.sh apk "$version" "$out" ;;
esac
