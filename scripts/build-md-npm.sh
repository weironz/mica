#!/usr/bin/env bash
# Build the npm package's wasm artifacts: the same crates/md-wasm engine, wrapped
# for `--target web` (bundlers/browsers) and `--target nodejs` (Node).
#
# The Flutter web editor uses a third packaging of the same crate
# (`--target no-modules`, scripts/build-md-wasm.sh). Same source, same version,
# different JS wrapper — never a second engine.
#
#   bash scripts/build-md-npm.sh          # writes packages/mica-md-js/dist/{web,node}
#
# dist/ is generated and .gitignore'd; npm scripts use it in place.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

bash scripts/check-md-wasm-toolchain.sh

CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 CARGO_PROFILE_RELEASE_PANIC=abort \
  cargo build -p mica-md-wasm --target wasm32-unknown-unknown --release --locked
target_dir=$(cargo metadata --format-version 1 --no-deps --locked | jq -er .target_directory)
wasm="$target_dir/wasm32-unknown-unknown/release/mica_md_wasm.wasm"

dist=packages/mica-md-js/dist
rm -rf "$dist"
mkdir -p "$dist/web" "$dist/node"
wasm-bindgen --target web --out-dir "$dist/web" --out-name mica_md \
  --remove-name-section --remove-producers-section "$wasm"
wasm-bindgen --target nodejs --out-dir "$dist/node" --out-name mica_md \
  --remove-name-section --remove-producers-section "$wasm"
# wasm-bindgen's nodejs target emits CommonJS, but the package is
# `"type": "module"` — without this marker Node would parse the glue as ESM.
printf '{ "type": "commonjs" }\n' > "$dist/node/package.json"

# Byte-size receipt (gzip per file, as an HTTP transfer would see it). The npm
# package has no hard budget of its own; the Flutter bundle's 262144 B gate in
# build-md-wasm.sh is the production budget. Report so drift is visible.
for dir in "$dist/web" "$dist/node"; do
  total=0
  for f in "$dir"/mica_md_bg.wasm "$dir"/mica_md.js; do
    [ -f "$f" ] || continue
    total=$((total + $(gzip -n -9 -c "$f" | wc -c)))
  done
  echo "==> $(basename "$dir") gzip wasm+js: $total B"
done
echo "==> npm artifacts in $dist"