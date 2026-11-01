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
# Exempt, with the reason:
#   lib/src/rust/            generated FFI bindings
#   lib/markdown/            the codec facade itself
#   lib/editor/highlight.dart    code-block syntax colouring (Markdown is one of
#                                the highlighted languages; colours, not structure)
#   lib/editor/paste_repair.dart paste heuristics, documented as not grammar
# Anything else that must touch Markdown syntax (keystroke input rules that
# cannot wait for a round trip) carries `md-syntax-ok: <reason>` on the same
# line or the line above. Review each new marker like a dependency.
set -euo pipefail
cd "$(dirname "$0")/.."

lib=${MICA_DART_LIB:-clients/mica_flutter/lib}  # override: scripts/test-check-dart-markdown.sh
# One ERE per line, literal (single-quoted heredoc: no shell escaping). Each
# matches the syntax inside a Dart string or regex literal, not code in general.
patterns=$(cat <<'ERE'
'\*\*'|"\*\*"
'~~'|"~~"
'```|"```
'!\[|"!\[|'\]\(|"\]\(|\\\]\\\(
'#{1,6} '|'> '|'- \[[ x]\]
'\[\^\$|"\[\^\$
'\\\\\$\{
'\$\{?[A-Za-z_]+\}?: |'  - 
ERE
)
joined=$(printf '%s' "$patterns" | paste -sd'|')

hits=$(grep -rnE "$joined" "$lib" --include='*.dart' \
  | grep -vE "^$lib/(src/rust/|markdown/|editor/highlight\.dart:|editor/paste_repair\.dart:)" \
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
  violations+="$hit"$'\n'
done <<< "$hits"

if [ -n "$violations" ]; then
  printf '%s' "$violations"
  echo "FAIL: Markdown syntax in Dart (see above). Route it through MarkdownCodec," >&2
  echo "      or, for a keystroke input rule, mark the line 'md-syntax-ok: <reason>'." >&2
  exit 1
fi
echo "==> no Markdown syntax in Dart outside the codec (lib/)"
