#!/usr/bin/env bash
# The ONE place the Linux .deb and Android APKs are built. Runs unchanged on a
# host with the toolchain (native) and inside the pinned builder image
# (deploy/Dockerfile.client-build), so the two paths cannot drift. Callers:
# scripts/build-linux-deb.sh and scripts/build-android-apk.sh.
#
#   bash scripts/client-package-inner.sh deb|apk <version> <out-dir>
#
# Outputs (names are what release manifests and release-publish expect):
#   deb  Mica-X.Y.Z-linux-amd64.deb
#   apk  Mica-X.Y.Z-android-arm64.apk, Mica-X.Y.Z-android-armv7.apk
set -euo pipefail
kind=${1:?deb|apk}; version=${2:?version}; out=$(realpath -m "${3:?out dir}")
cd "$(dirname "$0")/.."
flutter=${FLUTTER:-flutter}
mkdir -p "$out"
if [ "$kind" = apk ]; then
  source scripts/lib-local.sh
  source scripts/android-signing.sh
  require_android_signing
fi

# Android's versionCode must grow with every release: X.Y.Z -> X*10000+Y*100+Z
# (0.14.3 -> 1403). Linux ignores it but takes the same flags.
IFS=. read -r major minor patch <<<"${version%%[-+]*}"
code=$((10#$major * 10000 + 10#$minor * 100 + 10#$patch))
build_flags=(--release "--build-name=$version" "--build-number=$code")

cd clients/mica_flutter
case "$kind" in
  deb)
    "$flutter" build linux "${build_flags[@]}"
    bash ../../scripts/package-deb.sh "$version" build/linux/x64/release/bundle "$out"
    ;;
  apk)
    # Per-ABI APKs instead of one universal APK: ~45 MB each versus 136 MB,
    # since every ABI carries its own copy of the Rust core. Release signing
    # uses the stable caller-supplied key; no debug key is used for releases.
    "$flutter" build apk "${build_flags[@]}" --split-per-abi \
      --target-platform android-arm,android-arm64
    apks=build/app/outputs/flutter-apk
    cp "$apks/app-arm64-v8a-release.apk" "$out/Mica-$version-android-arm64.apk"
    cp "$apks/app-armeabi-v7a-release.apk" "$out/Mica-$version-android-armv7.apk"
    ;;
  *) echo "unknown package kind: $kind (deb|apk)" >&2; exit 2 ;;
esac
ls -l "$out"
