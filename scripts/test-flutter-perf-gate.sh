#!/usr/bin/env bash
# Keep strict timing benchmarks out of the highly parallel general test run.
# The budgets are unchanged; every entry point must select the same five cases
# and run them serially with a receipt.
set -euo pipefail
cd "$(dirname "$0")/.."
for entry in scripts/local-ci.sh scripts/release-check.sh .github/workflows/ci.yml; do
  grep -Fq 'flutter_receipt 1000 --exclude-tags perf test' "$entry" || {
    echo "FAIL: $entry no longer excludes perf cases from the general run" >&2
    exit 1
  }
  grep -Fq 'flutter_receipt 5 --tags perf --concurrency=1' "$entry" || {
    echo "FAIL: $entry no longer runs all five perf cases serially" >&2
    exit 1
  }
  grep -Fq 'test/markdown_codec_bench_test.dart test/perf/html_paste_bench_test.dart test/perf/table_cells_bench_test.dart' "$entry" || {
    echo "FAIL: $entry loads suites beyond the three timed benchmark files" >&2
    exit 1
  }
  general_line=$(grep -nF 'flutter_receipt 1000 --exclude-tags perf test' "$entry" | head -1 | cut -d: -f1)
  perf_line=$(grep -nF 'flutter_receipt 5 --tags perf --concurrency=1' "$entry" | head -1 | cut -d: -f1)
  [ "$perf_line" -lt "$general_line" ] || {
    echo "FAIL: $entry measures perf after the competing general run" >&2
    exit 1
  }
done
for suite in \
  clients/mica_flutter/test/markdown_codec_bench_test.dart \
  clients/mica_flutter/test/perf/html_paste_bench_test.dart \
  clients/mica_flutter/test/perf/table_cells_bench_test.dart; do
  grep -Fq "tags: 'perf'" "$suite" || {
    echo "FAIL: $suite lost its perf tag" >&2
    exit 1
  }
done
# Four declarations produce five cases: the HTML paste declaration runs for
# both 100 KB and 1024 KB inputs.
count=$(grep -Rho "tags: 'perf'" clients/mica_flutter/test --include='*_test.dart' | wc -l)
[ "$count" -eq 4 ] || {
  echo "FAIL: expected four timed perf declarations (five cases), found $count" >&2
  exit 1
}
echo 'PASS: all CI entry points exclude five timed cases from the parallel run and measure them serially with unchanged budgets'
