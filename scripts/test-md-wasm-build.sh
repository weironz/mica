#!/usr/bin/env bash
# Exercise the real production script using disposable toolchain fixtures.
# No Rust/Flutter build, server or user configuration is touched.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
fixture=$(mktemp -d)
trap 'rm -rf -- "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/bin" "$fixture/crates/md-core/tests/fixtures/wire"
cp "$repo/scripts/build-md-wasm.sh" "$repo/scripts/check-md-wasm-toolchain.sh" \
  "$repo/scripts/lib-local.sh" "$repo/scripts/lib-receipt.sh" "$fixture/scripts/"
printf '[[package]]\nname = "wasm-bindgen"\nversion = "0.2.129"\n' > "$fixture/Cargo.lock"
printf '{}\n' > "$fixture/crates/md-core/tests/fixtures/wire/vector.json"
cat > "$fixture/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> invocations
case "$1" in
  test)
    [ "${GEN_GOLD:-0}" != 1 ] || exit 91
    [ "${GOLD_FAIL:-0}" != 1 ] || exit 92
    printf 'test result: ok. %s passed; 0 failed; 0 ignored\n' "${GOLD_PASSED:-9}" ;;
  build)
    [ "$CARGO_PROFILE_RELEASE_LTO" = true ]
    [ "$CARGO_PROFILE_RELEASE_CODEGEN_UNITS" = 1 ]
    [ "$CARGO_PROFILE_RELEASE_PANIC" = abort ]
    mkdir -p target/wasm32-unknown-unknown/release
    printf wasm > target/wasm32-unknown-unknown/release/mica_md_wasm.wasm ;;
  metadata) printf '{"target_directory":"%s/target"}\n' "$PWD" ;;
  run) printf '0123456789abcdef\n' ;;
  *) exit 93 ;;
esac
STUB
cat > "$fixture/bin/wasm-bindgen" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
if [ "$1" = --version ]; then
  printf 'wasm-bindgen %s\n' "${CLI_VERSION:-0.2.129}"
  exit
fi
printf '%s\n' "$*" >> invocations
while [ "$#" -gt 0 ]; do
  if [ "$1" = --out-dir ]; then
    out=$2
    shift
  fi
  shift
done
printf wasm > "$out/mica_md_bg.wasm"
printf glue > "$out/mica_md.js"
STUB
chmod +x "$fixture/bin/cargo" "$fixture/bin/wasm-bindgen"
run() { (cd "$fixture" && PATH="$fixture/bin:$PATH" bash scripts/build-md-wasm.sh); }
refuses() {
  local label=$1
  shift
  if (for assignment in "$@"; do export "${assignment?}"; done; run) > "$fixture/output" 2>&1; then
    printf 'FAIL: %s passed unexpectedly\n' "$label" >&2
    exit 1
  fi
  printf 'PASS: %s refused\n' "$label"
}
refuses 'CLI mismatch' CLI_VERSION=0.2.128
refuses 'gold verification failure' GOLD_FAIL=1
refuses 'fewer than nine tests' GOLD_PASSED=8
refuses 'generation mode' GEN_GOLD=1
rm "$fixture/crates/md-core/tests/fixtures/wire/vector.json"
refuses 'empty gold corpus'
printf '{}\n' > "$fixture/crates/md-core/tests/fixtures/wire/vector.json"
run > "$fixture/output" 2>&1
cmp "$fixture/clients/mica_flutter/web/mica_md/SOURCE_FINGERPRINT" \
  "$fixture/clients/mica_flutter/test/support/mica_md/SOURCE_FINGERPRINT"
cmp "$fixture/crates/md-core/tests/fixtures/wire/vector.json" \
  "$fixture/clients/mica_flutter/test/support/mica_md/gold/vector.json"
grep -q '^test --locked -p mica-md --no-default-features --test wire_gold$' "$fixture/invocations"
grep -q '^build -p mica-md-wasm --target wasm32-unknown-unknown --release --locked$' "$fixture/invocations"
grep -q -- '--remove-name-section --remove-producers-section' "$fixture/invocations"
printf 'PASS: default gold verified, locked build flags, fingerprint and gold copied\n'
# Verify the actual recipe expands to a wasm build before the Chrome launch.
(cd "$repo" && just --dry-run app chrome) > "$fixture/app" 2>&1
grep -q 'build-md-wasm.sh' "$fixture/app"
printf 'PASS: Chrome app recipe includes the wasm prerequisite\n'
