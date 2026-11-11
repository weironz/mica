#!/usr/bin/env bash
# A large engine table must never prune valid local font shards (pipefail).
set -euo pipefail
cd "$(dirname "$0")/.."
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/tool/gfonts"
cp clients/mica_flutter/tool/gfonts/mirror.sh "$fixture/tool/gfonts/"
python3 - "$fixture" <<'PY'
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
engine = root / 'sdk/bin/cache/flutter_web_sdk/lib/_engine/engine'
(engine / 'canvaskit').mkdir(parents=True)
paths = [f'notosanssc/v37/{"a" * 140}.{i}.woff2' for i in range(800)]
roboto = 'roboto/v32/test.woff2'
(engine / 'font_fallback_data.dart').write_text('\n'.join(f"'{p}'" for p in paths))
(engine / 'canvaskit/fonts.dart').write_text(f"'{roboto}'")
for path in [*paths, roboto]:
    target = root / 'web/gfonts' / path
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(b'font fixture')
PY
export FLUTTER_ROOT="$fixture/sdk"
bash "$fixture/tool/gfonts/mirror.sh" > "$fixture/output"
[ "$(find "$fixture/web/gfonts" -name '*.woff2' | wc -l)" -eq 801 ] \
  || { cat "$fixture/output"; echo 'FAIL: valid font shards were pruned' >&2; exit 1; }
bash "$fixture/tool/gfonts/mirror.sh" --check > "$fixture/output"
rm "$fixture/web/gfonts/roboto/v32/test.woff2"
if bash "$fixture/tool/gfonts/mirror.sh" --check > "$fixture/output" 2>&1; then
  echo 'FAIL: a missing runtime font passed the mirror gate' >&2
  exit 1
fi
echo 'Font mirror: large tables preserve all 801 shards; missing files are refused'
