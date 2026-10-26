#!/usr/bin/env bash
# Run every browser-only Flutter suite in a real browser, with a receipt.
#
#   bash scripts/browser-tests.sh          # needs Chrome/Chromium ($CHROME_EXECUTABLE or on PATH)
#
# A browser suite under plain `flutter test` either skips (`@TestOn('browser')`)
# or compiles to its VM stub (the `if (dart.library.js_interop)` entry files) —
# both "pass" while testing nothing. Before this script only one of the five
# suites ran anywhere (CI, web_idb_doc_store); the other four never ran in a gate. Suites are found by
# those two markers, so a new one cannot be forgotten in a list.
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/lib-local.sh
source scripts/lib-receipt.sh

cd clients/mica_flutter
mapfile -t suites < <(grep -l -E "^@TestOn\('browser'\)|if \(dart\.library\.js_interop\) '[a-z_]+_test_web\.dart'" test/*_test.dart | sort)
[ "${#suites[@]}" -gt 0 ] || fail "found no browser suites — the discovery markers changed?"
for suite in "${suites[@]}"; do
  note "browser suite: $suite"
  flutter_receipt 1 --platform chrome "$suite"
done
