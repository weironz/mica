#!/usr/bin/env bash
# Refuse source files that .gitignore hides.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

# The npm package's wasm dist is built output (scripts/build-md-npm.sh).
allowed='^packages/mica-md-js/dist/'

hidden=$(git ls-files --others --ignored --exclude-standard -- \
  crates clients/mica_rust/src clients/mica_rust/tests clients/mica_rust/web \
  migrations scripts e2e packages \
  | grep -E '\.(rs|sql|sh|ps1|mjs|js|toml|html)$' \
  | grep -v -E '/(target|build|node_modules)/' \
  | grep -v -E "$allowed" || true)
[ -z "$hidden" ] || fail "source files hidden by .gitignore (commit them with a negation rule, or delete them):
$hidden"
note "no ignored source files"
