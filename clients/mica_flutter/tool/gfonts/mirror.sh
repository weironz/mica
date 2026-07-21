#!/usr/bin/env bash
# Same-origin mirror of the Flutter web engine's runtime font downloads.
#
# The web engine hits fonts.gstatic.com at runtime for two things, and
# --no-web-resources-cdn covers neither (that flag is CanvasKit-only):
#
#   1. the default text font ("Roboto", one woff2, fetched on EVERY page load
#      because no asset family is named exactly `Roboto`), and
#   2. lazy font-fallback shards (Noto *) for any glyph the style's font chain
#      doesn't cover — emoji, symbols, and every text run that misses the
#      cjkFontFallback wiring.
#
# Both URLs are built as `configuration.fontFallbackBaseUrl + <relative path>`.
# web/flutter_bootstrap.js points fontFallbackBaseUrl at "gfonts/", and this
# script keeps clients/mica_flutter/web/gfonts/ holding exactly the files the
# INSTALLED engine can request for the families below (URL paths are versioned,
# e.g. notosanssc/v37/…; a Flutter upgrade can move them — CI runs --check so
# that drift fails the build instead of 404ing at runtime).
#
# Mirrored: EVERY family in the engine's tables (262 files, ~11.2 MB) except
# the four CJK siblings JP/KR/TC/HK (462 files, ~9.5 MB more): their repertoire
# is pan-CJK so Noto Sans SC + the bundled DroidSansFallback tail already cover
# those glyphs, and with a zh locale the engine's greedy picker prefers the SC
# shards anyway. A request for an excluded family 404s same-origin and the run
# degrades to the styled chain / tofu — the pre-mirror offline behavior. A
# failed family is retried on every relayout (the engine never marks it done),
# so DON'T exclude anything a real document can plausibly need: e.g. plain
# `notosans` is the greedy winner for stray symbols like ∑ √ and its absence
# spams 404s on every keystroke (measured). Exclusion list only — new families
# from an engine bump are mirrored automatically on the next sync.
#
# Usage (from anywhere; paths are script-relative):
#   tool/gfonts/mirror.sh          # sync: download missing shards, prune stale
#   tool/gfonts/mirror.sh --check  # verify only (CI); exit 1 when out of sync
set -euo pipefail

EXCLUDE='notosansjp notosanskr notosanstc notosanshk'
GSTATIC='https://fonts.gstatic.com/s'

APP_ROOT=$(cd "$(dirname "$0")/../.." && pwd)   # clients/mica_flutter
MIRROR="$APP_ROOT/web/gfonts"

# The engine sources shipped with the installed Flutter (populated by any web
# build / `flutter precache --web`). We read the same data tables the engine
# compiles in, so the expected-file list is exact by construction.
if [ -z "${FLUTTER_ROOT:-}" ]; then
  flutter_bin=$(command -v flutter) || { echo "mirror.sh: flutter not on PATH and FLUTTER_ROOT unset" >&2; exit 1; }
  # bin/flutter -> sdk root (resolve one symlink level for e.g. homebrew shims)
  [ -L "$flutter_bin" ] && flutter_bin=$(readlink -f "$flutter_bin")
  FLUTTER_ROOT=$(cd "$(dirname "$flutter_bin")/.." && pwd)
fi
ENGINE="$FLUTTER_ROOT/bin/cache/flutter_web_sdk/lib/_engine/engine"
DATA="$ENGINE/font_fallback_data.dart"
ROBOTO_SRC="$ENGINE/canvaskit/fonts.dart"
for f in "$DATA" "$ROBOTO_SRC"; do
  [ -f "$f" ] || { echo "mirror.sh: $f not found — run a web build (or flutter precache --web) first, or point FLUTTER_ROOT at the SDK the build uses" >&2; exit 1; }
done

# Expected relative paths: the hardcoded default-Roboto woff2 + every shard of
# every non-excluded family. Paths look like notosanssc/v37/<hash>.4.woff2.
excl_re=$(echo "$EXCLUDE" | tr ' ' '|')
expected=$( {
  grep -oE "roboto/v[0-9]+/[A-Za-z0-9_-]+\.woff2" "$ROBOTO_SRC" | head -1
  grep -oE "'[a-z0-9]+/v[0-9]+/[^']+\.woff2'" "$DATA" | tr -d "'" | grep -Ev "^($excl_re)/"
} | sort -u )
[ -n "$expected" ] || { echo "mirror.sh: extracted no font URLs — engine data format changed?" >&2; exit 1; }

missing=$(echo "$expected" | while IFS= read -r p; do [ -s "$MIRROR/$p" ] || echo "$p"; done)
present=$(cd "$MIRROR" 2>/dev/null && find . -name '*.woff2' | sed 's|^\./||' || true)
stale=$(echo "$present" | while IFS= read -r p; do
  [ -n "$p" ] || continue
  echo "$expected" | grep -qxF "$p" || echo "$p"
done)

if [ "${1:-}" = "--check" ]; then
  [ -n "$stale" ] && { echo "mirror.sh: stale files no engine URL references (prune with a sync run):"; echo "$stale" | sed 's/^/  /'; }
  if [ -n "$missing" ]; then
    echo "mirror.sh: MISSING from web/gfonts (engine would 404 at runtime):"
    echo "$missing" | sed 's/^/  /'
    echo "mirror.sh: web/gfonts is out of sync with this Flutter's engine — run tool/gfonts/mirror.sh with the same Flutter version and commit the result" >&2
    exit 1
  fi
  echo "mirror.sh: OK — $(echo "$expected" | wc -l) files match the installed engine"
  exit 0
fi

n=0
if [ -n "$missing" ]; then
  while IFS= read -r p; do
    echo "  GET $p"
    curl -fsSL --retry 3 --create-dirs -o "$MIRROR/$p" "$GSTATIC/$p"
    n=$((n + 1))
  done <<EOF
$missing
EOF
fi
if [ -n "$stale" ]; then
  while IFS= read -r p; do
    echo "  RM  $p"
    rm -f "$MIRROR/$p"
  done <<EOF
$stale
EOF
  # drop now-empty version dirs left behind by an engine bump
  find "$MIRROR" -type d -empty -delete 2>/dev/null || true
fi
total=$(echo "$expected" | wc -l)
size=$(du -sh "$MIRROR" 2>/dev/null | cut -f1)
echo "mirror.sh: synced — downloaded $n, mirror holds $total files ($size)"
