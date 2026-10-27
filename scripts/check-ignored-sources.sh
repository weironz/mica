#!/usr/bin/env bash
# Refuse source files that .gitignore hides.
#
# An ignored file never shows in `git status`, so it cannot be committed by
# accident — and it also cannot be committed on purpose without noticing. If
# the code that is committed references it (`mod`, `import`, a migration), the
# author's machine builds and every clean checkout does not: the gate passed on
# a tree nobody else has. Happened twice with one `*credential*` rule.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh

# Generated on every machine by `flutter gen-l10n`; ignored on purpose.
allowed='^clients/mica_flutter/lib/l10n/app_localizations(_[a-z]+)?\.dart$|^clients/mica_flutter/(web/mica_md|test/support/mica_md)/'

hidden=$(git ls-files --others --ignored --exclude-standard -- \
  crates clients/mica_flutter/lib clients/mica_flutter/test clients/mica_flutter/web \
  clients/mica_flutter/integration_test clients/mica_flutter/rust/src \
  clients/mica_flutter/windows/runner migrations scripts e2e \
  | grep -E '\.(rs|dart|sql|sh|ps1|mjs|js|cpp|h|toml)$' \
  | grep -v -E '/(target|build|node_modules|\.dart_tool)/' \
  | grep -v -E "$allowed" || true)
[ -z "$hidden" ] || fail "source files hidden by .gitignore (commit them with a negation rule, or delete them):
$hidden"
note "no ignored source files"
