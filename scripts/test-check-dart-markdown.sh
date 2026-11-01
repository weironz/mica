#!/usr/bin/env bash
# Proves scripts/check-dart-markdown.sh FAILS on the writers it exists to stop
# — a gate that passes on everything looks identical to one that works. The
# probes are trimmed from the two Dart writers that slipped past the G4 review
# (the HTML paste converter and the page-properties mirror).
set -euo pipefail
cd "$(dirname "$0")/.."
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/editor" "$fixture/markdown"

run() { MICA_DART_LIB="$fixture" bash scripts/check-dart-markdown.sh > "$fixture/out" 2>&1; }
expect_fail() {
  if run; then
    echo "FAIL: guard passed $1" >&2
    cat "$fixture/out" >&2
    exit 1
  fi
  echo "PASS: guard refused $1"
}

cat > "$fixture/editor/clean.dart" <<'DART'
String greet(String name) => 'hello $name';
/// Doc comments may quote syntax: `**bold**`, `[x](y)`.
DART
run || { cat "$fixture/out" >&2; echo "FAIL: guard refused clean code" >&2; exit 1; }
echo "PASS: clean code accepted"

cat > "$fixture/editor/writer.dart" <<'DART'
String wrap(String s) => '**' + s + '**';
DART
expect_fail "an emphasis writer"

cat > "$fixture/editor/writer.dart" <<'DART'
String esc(String s) =>
    s.replaceAllMapped(RegExp(r'[\\`*_~\[\]<$]'), (m) => '\\${m[0]}');
DART
expect_fail "a Markdown escaper"

cat > "$fixture/editor/writer.dart" <<'DART'
String link(String t, String u) => '[$t](${u})' + '![x](y)';
DART
expect_fail "a link/image writer"

cat > "$fixture/editor/writer.dart" <<'DART'
List<String> render(String key, String v) => ['$key: $v', '  - $v'];
DART
expect_fail "a front-matter writer"

cat > "$fixture/editor/writer.dart" <<'DART'
// md-syntax-ok: keystroke input rule
bool quote(String t) => t.startsWith('> ');
DART
run || { cat "$fixture/out" >&2; echo "FAIL: guard refused a marked input rule" >&2; exit 1; }
echo "PASS: a line marked md-syntax-ok is accepted"

cat > "$fixture/editor/writer.dart" <<'DART'
final heading = RegExp(r'^#{1,6}\s+(.*)$');
final bold = RegExp(r'\*\*(.+?)\*\*');
DART
expect_fail "a RegExp Markdown reader (raw-string form)"

cat > "$fixture/editor/writer.dart" <<'DART'
bool isTable(String md) => md.contains('| --- ');
DART
expect_fail "a table delimiter-row reader"

cat > "$fixture/editor/writer.dart" <<'DART'
// md-syntax-ok-begin: colours, not structure
final rules = [r'#{1,6}[^\n]*', r'(?:```|~~~)'];
// md-syntax-ok-end
DART
run || { cat "$fixture/out" >&2; echo "FAIL: guard refused a marked region" >&2; exit 1; }
echo "PASS: a md-syntax-ok-begin/end region is accepted"

cat >> "$fixture/editor/writer.dart" <<'DART'
final after = RegExp(r'\*\*x\*\*');
DART
expect_fail "syntax after a closed region"

rm "$fixture/editor/writer.dart"
printf "String wrap(String s) => '**' + s;\n" > "$fixture/markdown/codec.dart"
run || { cat "$fixture/out" >&2; echo "FAIL: guard refused the exempt codec dir" >&2; exit 1; }
echo "PASS: the codec directory is exempt"
