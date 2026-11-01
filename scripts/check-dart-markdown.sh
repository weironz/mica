#!/usr/bin/env bash
# Markdown has exactly one implementation, in Rust (crates/md-core); Dart calls
# it through MarkdownCodec. This gate fails when Dart under lib/ starts to READ
# or WRITE Markdown syntax again: emphasis / fence / link markers, block
# leaders, footnote refs, backslash-escaping of Markdown punctuation, and
# front-matter lines (`key: value`, `  - item`).
#
# Why a pattern gate and not a list of banned function names: the G4 review
# checked for the retired mirror's function names, and so missed two Markdown
# writers that had different names (the page-properties mirror and the HTML
# paste converter). This looks at what code DOES, by the syntax it handles.
#
# Exempt directories, with the reason:
#   lib/src/rust/   generated FFI bindings
#   lib/markdown/   the codec facade itself
#   lib/l10n/       generated from .arb (AI prompt templates fence the code they
#                   send); generated files cannot carry markers
# No file is exempt as a whole. Code that must touch Markdown syntax (keystroke
# input rules that cannot wait for a round trip, the code highlighter's rules
# for Markdown as a language) carries `md-syntax-ok: <reason>` on the same line
# or the line above, or sits between `md-syntax-ok-begin: <reason>` and
# `md-syntax-ok-end`. Review each new marker like a dependency.
set -euo pipefail
cd "$(dirname "$0")/.."

lib=${MICA_DART_LIB:-clients/mica_flutter/lib}  # override: scripts/test-check-dart-markdown.sh
# One ERE per line, literal (single-quoted heredoc: no shell escaping). Each
# matches the syntax inside a Dart string or regex literal, not code in general.
# The last line catches the regex-escaped forms (`r'\*\*'`, `#{1,6}`) and
# fences anywhere: a Dart Markdown reader is usually a RegExp, not a string.
# `| ---` is a table delimiter row (a reader of the engine's output is still a
# reader).
patterns=$(cat <<'ERE'
'\*\*'|"\*\*"
'~~'|"~~"
'!\[|"!\[|'\]\(|"\]\(|\\\]\\\(
'#{1,6} '|'> '|'- \[[ x]\]
'\[\^\$|"\[\^\$
'\\\\\$\{
'\$\{?[A-Za-z_]+\}?: |'  - 
\\\*\\\*|#\{1,6\}|```|~~~
['"]\| ?---
ERE
)
joined=$(printf '%s' "$patterns" | paste -sd'|')

hits=$(grep -rnE "$joined" "$lib" --include='*.dart' \
  | grep -vE "^$lib/(src/rust/|markdown/|l10n/)" \
  || true)

violations=""
while IFS= read -r hit; do
  [ -n "$hit" ] || continue
  file=${hit%%:*}
  rest=${hit#*:}
  line=${rest%%:*}
  # Comments and doc comments may quote syntax freely.
  text=${rest#*:}
  if [[ "$text" =~ ^[[:space:]]*/// ]] || [[ "$text" =~ ^[[:space:]]*// ]]; then
    continue
  fi
  if grep -q 'md-syntax-ok:' <(sed -n "$((line > 1 ? line - 1 : 1)),${line}p" "$file"); then
    continue
  fi
  # Inside a begin/end region: the last region marker above the hit is a begin.
  last=$(sed -n "1,${line}p" "$file" | grep -oE 'md-syntax-ok-(begin|end)' | tail -n1 || true)
  if [ "$last" = md-syntax-ok-begin ]; then
    continue
  fi
  violations+="$hit"$'\n'
done <<< "$hits"

if [ -n "$violations" ]; then
  printf '%s' "$violations"
  echo "FAIL: Markdown syntax in Dart (see above). Route it through MarkdownCodec," >&2
  echo "      or, for a keystroke input rule, mark the line 'md-syntax-ok: <reason>'." >&2
  exit 1
fi
echo "==> no Markdown syntax in Dart outside the codec (lib/)"
