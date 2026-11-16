#!/usr/bin/env bash
# Run every browser-only Flutter suite in a real browser, with a receipt.
#
#   bash scripts/browser-tests.sh          # needs Chrome/Chromium ($CHROME_EXECUTABLE or on PATH)
#   bash scripts/browser-tests.sh --report-status
#       ...and, when every suite passed on a clean checkout, record the commit
#       status `mica/browser-tests` = success on HEAD. That status is what
#       release-publish and the Release workflow require (docs/ops/release.md).
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

report=0
for a in "$@"; do
  case "$a" in
    --report-status) report=1 ;;
    *) fail "unknown flag $a" ;;
  esac
done
if [ "$report" = 1 ]; then
  # The status vouches for a COMMIT, so the tree must be exactly that commit.
  git diff --quiet && git diff --cached --quiet \
    || fail "--report-status needs a clean checkout: the status would vouch for a commit you did not test"
  head_sha=$(git rev-parse HEAD)
  require_gh_api
fi
bash scripts/build-md-wasm.sh
bash scripts/build-doc-wasm.sh
bash scripts/build-preview-wasm.sh

cd clients/mica_flutter
# Clean runners do not have the generated localization files used by the suites.
"$FLUTTER" pub get --enforce-lockfile
"$FLUTTER" gen-l10n
mapfile -t suites < <(grep -l -E "^@TestOn\('browser'\)|if \(dart\.library\.js_interop\) '[a-z_]+_test_web\.dart'" test/*_test.dart | sort)
[ "${#suites[@]}" -gt 0 ] || fail "found no browser suites — the discovery markers changed?"
for suite in "${suites[@]}"; do
  note "browser suite: $suite"
  flutter_receipt 1 --no-pub --platform chrome "$suite"
done

if [ "$report" = 1 ]; then
  report_browser_status "$head_sha" "${#suites[@]} browser suites passed on $(host_os)"
fi
