#!/usr/bin/env bash
# Validate absent, partial, empty and malformed config without creating a key.
set -euo pipefail
cd "$(dirname "$0")/.."
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
printf 'not a real signing key\n' > "$fixture/dummy.keystore"
run() { (source scripts/lib-local.sh; source scripts/android-signing.sh; require_android_signing); }
unset MICA_ANDROID_KEYSTORE_PATH MICA_ANDROID_KEY_ALIAS MICA_ANDROID_STORE_PASSWORD MICA_ANDROID_KEY_PASSWORD MICA_ANDROID_STORE_TYPE
if run > "$fixture/log" 2>&1; then echo 'FAIL missing key accepted'; exit 1; fi
export MICA_ANDROID_KEYSTORE_PATH="$fixture/dummy.keystore" MICA_ANDROID_KEY_ALIAS=test
export MICA_ANDROID_STORE_PASSWORD=fixture MICA_ANDROID_KEY_PASSWORD=fixture
run
MICA_ANDROID_STORE_TYPE=PKCS12 run
if MICA_ANDROID_KEY_PASSWORD= run > "$fixture/log" 2>&1; then echo 'FAIL empty key password accepted'; exit 1; fi
if MICA_ANDROID_KEYSTORE_PATH="$fixture/missing" run > "$fixture/log" 2>&1; then echo 'FAIL missing file accepted'; exit 1; fi
if MICA_ANDROID_STORE_TYPE=invalid run > "$fixture/log" 2>&1; then echo 'FAIL invalid type accepted'; exit 1; fi
if MICA_ANDROID_STORE_PASSWORD='   ' run > "$fixture/log" 2>&1; then echo 'FAIL blank password accepted'; exit 1; fi
echo 'Android signing preflight: complete configs accepted; absent/empty/blank/file/type errors refused (no key generated)'
