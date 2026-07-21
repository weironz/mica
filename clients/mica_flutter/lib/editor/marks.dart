import 'package:flutter/widgets.dart';
import 'model.dart' show kMonoFont;

/// Inline rich-text marks stored additively over a block's plain text (in
/// `data.marks`). The text stays clean; a mark is a `[start, end)` range with a
/// type (and an href for links). See docs/editor-engine.md.
class Mark {
  Mark(this.start, this.end, this.type, {this.href, this.title});

  final int start;
  final int end;
  final String type; // bold | italic | code | strike | link
  final String? href;

  /// Optional link title (`[t](url "title")`) — carried for GFM fidelity.
  final String? title;

  Mark shifted(int delta) =>
      Mark(start + delta, end + delta, type, href: href, title: title);

  Map<String, dynamic> toJson() => {
    'start': start,
    'end': end,
    'type': type,
    if (href != null) 'href': href,
    if (title != null) 'title': title,
  };

  static const types = {'bold', 'italic', 'code', 'strike', 'link', 'footnote'};
}

List<Mark> marksFromData(Map<String, dynamic> data) {
  final raw = data['marks'];
  if (raw is! List) return [];
  final marks = <Mark>[];
  for (final m in raw) {
    if (m is Map) {
      final start = (m['start'] as num?)?.toInt();
      final end = (m['end'] as num?)?.toInt();
      final type = m['type'] as String?;
      if (start != null && end != null && type != null && end > start) {
        marks.add(
          Mark(
            start,
            end,
            type,
            href: m['href'] as String?,
            title: m['title'] as String?,
          ),
        );
      }
    }
  }
  return marks;
}

List<Map<String, dynamic>> marksToJson(List<Mark> marks) => [
  for (final m in marks) m.toJson(),
];

const Color _codeColor = Color(0xFF334155);
const Color _linkColor = Color(0xFF2563EB);
const Color _mathColor = Color(0xFF7C3AED);
const Color _mathBg = Color(0x147C3AED);
const Color _footnoteColor = Color(0xFF2563EB);

/// Build a styled [TextSpan] for [text] with [marks] applied over [base].
TextSpan buildMarkedSpan(String text, List<Mark> marks, TextStyle base) {
  if (text.isEmpty) return TextSpan(text: '​', style: base);
  if (marks.isEmpty) return TextSpan(text: text, style: base);

  final len = text.length;
  final points = <int>{0, len};
  for (final m in marks) {
    points.add(m.start.clamp(0, len));
    points.add(m.end.clamp(0, len));
  }
  final sorted = points.toList()..sort();

  final children = <TextSpan>[];
  for (var i = 0; i < sorted.length - 1; i++) {
    final a = sorted[i];
    final b = sorted[i + 1];
    if (a >= b) continue;
    final active = marks.where((m) => m.start <= a && m.end >= b);
    var style = base;
    final decorations = <TextDecoration>[];
    for (final m in active) {
      switch (m.type) {
        case 'bold':
          // w600, matching the headings. w700 read too heavy in body text —
          // though see the caveat in inline_code_style_test.dart: for CJK this
          // may not change anything, because we bundle only a Regular face and
          // both weights land on the same synthesized bold.
          style = style.copyWith(fontWeight: FontWeight.w600);
        case 'italic':
          style = style.copyWith(fontStyle: FontStyle.italic);
        case 'code':
          // The pill background is drawn in the render layer (_paintInlineCode)
          // for rounded corners + padding; here just the mono font + calm ink.
          style = style.copyWith(fontFamily: kMonoFont, color: _codeColor);
        case 'strike':
          decorations.add(TextDecoration.lineThrough);
        case 'link':
          style = style.copyWith(color: _linkColor);
          decorations.add(TextDecoration.underline);
        case 'math':
          // LaTeX source shown styled until real typesetting lands.
          style = style.copyWith(
            fontFamily: kMonoFont,
            fontStyle: FontStyle.italic,
            color: _mathColor,
            backgroundColor: _mathBg,
          );
        case 'footnote':
          // GFM reference chip: small superscript label in link blue. The
          // raised baseline + reduced size reads as a footnote marker.
          style = style.copyWith(
            color: _footnoteColor,
            fontSize: (style.fontSize ?? 14) * 0.75,
            fontFeatures: const [FontFeature.superscripts()],
          );
      }
    }
    if (decorations.isNotEmpty) {
      style = style.copyWith(decoration: TextDecoration.combine(decorations));
    }
    children.add(TextSpan(text: text.substring(a, b), style: style));
  }
  return TextSpan(style: base, children: children);
}

/// Whether `[from, to)` is fully covered by a mark of [type].
bool rangeHasMark(List<Mark> marks, int from, int to, String type) {
  return marks.any((m) => m.type == type && m.start <= from && m.end >= to);
}

/// The math run a caret at [caret] sits inside, or null — what decides whether
/// the editor floats a typeset preview over the formula you are editing
/// (`_paintMathPreview` in render.dart).
///
/// STRICTLY inside. A caret on an edge belongs to the text as much as to the
/// formula, and calling that a hit would pop the card open every time you typed
/// your way past one. `$x$` (nothing between the delimiters) can never match,
/// which is right: there is no formula there to preview.
Mark? mathRunAt(List<Mark> marks, int caret) {
  for (final m in marks) {
    if (m.type == 'math' && caret > m.start && caret < m.end) return m;
  }
  return null;
}

/// A typeset inline formula is an atom: the caret rests on either side of it
/// but never inside its source. This maps an offset that landed inside a math
/// run to the run edge it should snap to.
///
/// [prefEnd] biases a caret exactly ambiguous between edges toward the end
/// (a range endpoint extending forward wants the far edge); a plain caret
/// snaps to the nearer edge. Offsets outside every run pass through unchanged,
/// so this is safe to call on every selection.
int snapOutOfMathRun(List<Mark> marks, int offset, {bool? prefEnd}) {
  final run = mathRunAt(marks, offset);
  if (run == null) return offset;
  if (prefEnd != null) return prefEnd ? run.end : run.start;
  return (offset - run.start) <= (run.end - offset) ? run.start : run.end;
}

/// The math run whose source ends exactly at [caret] (caret == run.end), or
/// null. Backspace at such a caret deletes the whole formula rather than a
/// single source character — the atom is indivisible. The mirror for Delete is
/// a run with `run.start == caret`.
Mark? mathRunEndingAt(List<Mark> marks, int caret) {
  for (final m in marks) {
    if (m.type == 'math' && m.end == caret && m.end > m.start) return m;
  }
  return null;
}

Mark? mathRunStartingAt(List<Mark> marks, int caret) {
  for (final m in marks) {
    if (m.type == 'math' && m.start == caret && m.end > m.start) return m;
  }
  return null;
}

/// Add or remove [type] over `[from, to)` (toggle decided by the caller).
List<Mark> applyMark(
  List<Mark> marks,
  int from,
  int to,
  String type, {
  String? href,
  required bool add,
}) {
  if (to <= from) return marks;
  final result = <Mark>[];
  for (final m in marks) {
    if (m.type != type || m.end <= from || m.start >= to) {
      result.add(m);
      continue;
    }
    // Trim the overlapping part of the same-type mark out of [from, to).
    if (m.start < from) result.add(Mark(m.start, from, type, href: m.href));
    if (m.end > to) result.add(Mark(to, m.end, type, href: m.href));
  }
  if (add) result.add(Mark(from, to, type, href: href));
  return _normalize(result);
}

/// Shift mark offsets for a text edit that replaced `[editStart, editOldEnd)`
/// with `delta = newLen - oldLen` net characters.
List<Mark> shiftMarks(
  List<Mark> marks,
  int editStart,
  int editOldEnd,
  int delta,
  int newLen,
) {
  int move(int x) {
    if (x <= editStart) return x;
    if (x >= editOldEnd) return x + delta;
    return editStart; // endpoint inside the replaced region collapses
  }

  final result = <Mark>[];
  for (final m in marks) {
    final s = move(m.start).clamp(0, newLen);
    final e = move(m.end).clamp(0, newLen);
    if (e > s) result.add(Mark(s, e, m.type, href: m.href));
  }
  return _normalize(result);
}

/// Marks for the two halves of a text split at [at]: first list keeps
/// `[0, at)` ranges, second gets `[at, …)` ranges rebased to 0. A mark
/// spanning the split is divided.
(List<Mark>, List<Mark>) splitMarks(List<Mark> marks, int at) {
  final before = <Mark>[];
  final after = <Mark>[];
  for (final m in marks) {
    if (m.start < at) {
      final end = m.end < at ? m.end : at;
      if (end > m.start) before.add(Mark(m.start, end, m.type, href: m.href));
    }
    if (m.end > at) {
      final start = m.start > at ? m.start : at;
      if (m.end > start) {
        after.add(Mark(start - at, m.end - at, m.type, href: m.href));
      }
    }
  }
  return (before, after);
}

/// Marks of two concatenated texts: [a]'s marks plus [b]'s shifted past the
/// [junction] (the first text's length).
List<Mark> concatMarks(List<Mark> a, List<Mark> b, int junction) =>
    _normalize([...a, for (final m in b) m.shifted(junction)]);

List<Mark> _normalize(List<Mark> marks) {
  if (marks.isEmpty) return marks;
  final sorted = [...marks]
    ..sort((a, b) {
      if (a.type != b.type) return a.type.compareTo(b.type);
      return a.start.compareTo(b.start);
    });
  final out = <Mark>[];
  for (final m in sorted) {
    if (m.end <= m.start) continue;
    if (out.isNotEmpty &&
        out.last.type == m.type &&
        out.last.href == m.href &&
        out.last.end >= m.start) {
      final prev = out.removeLast();
      out.add(
        Mark(
          prev.start,
          m.end > prev.end ? m.end : prev.end,
          m.type,
          href: m.href,
        ),
      );
    } else {
      out.add(m);
    }
  }
  return out;
}

const String _asciiPunct = r'''!"#$%&'()*+,-./:;<=>?@[\]^_`{|}~''';

/// Single-line link reference definition ` [label]: dest "title"`.
({String label, String dest, String? title})? parseRefDefinition(String raw) {
  final lead = raw.length - raw.trimLeft().length;
  if (lead > 3) return null;
  final t = raw.trim();
  if (!t.startsWith('[')) return null;
  final close = matchingBracket(t, 0);
  if (close < 0 || close + 1 >= t.length || t[close + 1] != ':') return null;
  final label = t.substring(1, close);
  if (label.trim().isEmpty) return null;
  // Labels may not contain unescaped brackets.
  for (var k = 1; k < close; k++) {
    if ((t[k] == '[' || t[k] == ']') && t[k - 1] != r'\') return null;
  }
  final rest = t.substring(close + 2).trim();
  if (rest.isEmpty) return null;
  // Reuse the suffix parser by appending a virtual `)` and requiring full
  // consumption.
  final suffix = parseLinkSuffix('$rest)', 0);
  if (suffix == null || suffix.next != rest.length + 1) return null;
  return (label: label, dest: suffix.dest, title: suffix.title);
}

/// Case-fold and collapse internal whitespace (spec label matching). Reference
/// labels match under Unicode CASE FOLDING, not mere lowercase: ẞ/ß and SS all
/// collapse to "ss". Mirrors Rust `normalize_label` (crates/markdown/src/lib.rs)
/// — the missing `ß`→`ss` fold made `[ß]` fail to resolve against `[ss]: /url`
/// on the Dart side only (P1-2, confirmed drift).
String normalizeLabel(String label) =>
    label.trim().split(RegExp(r'\s+')).join(' ').toLowerCase().replaceAll('ß', 'ss');

/// The `]` matching the `[` at [open], honoring nesting and escapes; -1 if
/// none.
int matchingBracket(String src, int open) {
  var depth = 0;
  var j = open;
  while (j < src.length) {
    final c = src[j];
    if (c == r'\' && j + 1 < src.length) {
      j += 2;
      continue;
    }
    if (c == '[') depth++;
    if (c == ']') {
      depth--;
      if (depth == 0) return j;
    }
    j++;
  }
  return -1;
}

const kDollar = r'$';

/// One past the closing backtick run of the code span opening at [start] (the
/// index of its first backtick), or -1 when the run never closes — CommonMark
/// leaves an unclosed run as literal text.
///
/// Mirrors `code_span_close` in `crates/markdown/src/lib.rs` (Rust is
/// authoritative — CLAUDE.md #2). Deliberately separate from the inline-code
/// scanner, which also strips padding and folds newlines; this only needs the
/// extent.
int _codeSpanClose(String src, int start) {
  var n = 0;
  while (start + n < src.length && src[start + n] == '`') {
    n++;
  }
  var j = start + n;
  while (j < src.length) {
    if (src[j] == '`') {
      var m = 0;
      while (j + m < src.length && src[j + m] == '`') {
        m++;
      }
      if (m == n) return j + m;
      j += m;
    } else {
      j++;
    }
  }
  return -1;
}

/// A valid `\$` math closer per the Pandoc rules, or -1.
int _findMathCloser(String src, int contentStart) {
  if (contentStart >= src.length) return -1;
  final first = src[contentStart];
  if (first == ' ' || first == '\t' || first == '\n') return -1;
  var j = contentStart;
  while (j < src.length) {
    final c = src[j];
    if (c == '\n') return -1; // inline math stays on one line
    if (c == '`') {
      // Code spans bind tighter than math — CommonMark 0.31.2 §6.1 gives them
      // higher precedence than every inline construct but HTML tags and
      // autolinks — so a `$` inside one is literal and cannot close us. Step
      // over the span whole. An unclosed run is literal text: fall through.
      final after = _codeSpanClose(src, j);
      if (after >= 0) {
        if (src.substring(j, after).contains('\n')) return -1;
        j = after;
        continue;
      }
    }
    if (c == kDollar && j > contentStart) {
      final prev = src[j - 1];
      if (prev == ' ' || prev == '\t' || prev == r'\') {
        j++;
        continue;
      }
      if (j + 1 < src.length && RegExp(r'[0-9]').hasMatch(src[j + 1])) {
        j++;
        continue;
      }
      return j;
    }
    j++;
  }
  return -1;
}

/// Weave inline math (`$…$` per Pandoc rules, and `\(…\)`) out of [src] into
/// `math` marks, leaving every other character VERBATIM — no other Markdown is
/// interpreted. The returned text has the delimiters stripped and the marks
/// cover each formula's run.
///
/// This is the inline counterpart to the display delimiters (`$$…$$`, `\[…\]`)
/// the paste path turns into a standalone math block: a pasted `$E=mc^2$`
/// belongs in the text flow as an inline formula, not on its own line.
({String text, List<Mark> marks}) parseInlineMath(String src) {
  final out = StringBuffer();
  final marks = <Mark>[];
  var i = 0;
  while (i < src.length) {
    // Inline math, LaTeX form: \( … \).
    if (src[i] == r'\' && i + 1 < src.length && src[i + 1] == '(') {
      final close = src.indexOf(r'\)', i + 2);
      if (close > i) {
        final inner = src.substring(i + 2, close).trim();
        if (inner.isNotEmpty) {
          final start = out.length;
          out.write(inner);
          marks.add(Mark(start, out.length, 'math'));
          i = close + 2;
          continue;
        }
      }
    }
    // Inline math, dollar form (not `$$`, which is a display block — the
    // opener must neither follow nor precede another `$`).
    if (src[i] == kDollar &&
        (i == 0 || src[i - 1] != kDollar) &&
        (i + 1 >= src.length || src[i + 1] != kDollar)) {
      final close = _findMathCloser(src, i + 1);
      if (close > 0) {
        final inner = src.substring(i + 1, close);
        final start = out.length;
        out.write(inner);
        marks.add(Mark(start, out.length, 'math'));
        i = close + 1;
        continue;
      }
    }
    out.write(src[i]);
    i++;
  }
  return (text: out.toString(), marks: marks);
}

/// Delimiter-INCLUSIVE spans of every valid inline math run in [src] — `$…$`
/// per the Pandoc rules and `\(…\)` — recognizing exactly what
/// [parseInlineMath] would parse, without touching the string.
///
/// The HTML paste path uses this to let real formulas through its Markdown
/// escaping: `htmlToMarkdown` backslash-escapes `$` (and `\`) in page text so
/// literal punctuation cannot re-parse as syntax, which silently killed every
/// pasted `$\eta = 2$` — LLM answers copied from a browser are HTML, so the
/// user's primary math workflow never produced a single math mark. Runs found
/// here pass through verbatim; a `$` with no valid closer (currency, a stray
/// dollar) still gets escaped, which is STRICTER than the plain-text paste.
List<({int start, int end})> mathRunSpans(String src) {
  final spans = <({int start, int end})>[];
  var i = 0;
  while (i < src.length) {
    if (src[i] == r'\' && i + 1 < src.length && src[i + 1] == '(') {
      final close = src.indexOf(r'\)', i + 2);
      if (close > i && src.substring(i + 2, close).trim().isNotEmpty) {
        spans.add((start: i, end: close + 2));
        i = close + 2;
        continue;
      }
    }
    if (src[i] == kDollar &&
        (i == 0 || src[i - 1] != kDollar) &&
        (i + 1 >= src.length || src[i + 1] != kDollar)) {
      final close = _findMathCloser(src, i + 1);
      if (close > 0) {
        spans.add((start: i, end: close + 1));
        i = close + 1;
        continue;
      }
    }
    i++;
  }
  return spans;
}

/// GFM extended autolink starting at src[i] (caller checks the word
/// boundary): bare http(s)/ftp URLs, `www.` (href gains http://), bare
/// emails. Returns (consumed chars, href), or null.
(int, String)? extendedAutolink(String src, int i) {
  final rest = src.substring(i);
  final lower = rest.toLowerCase();
  for (final (prefix, implied) in [
    ('https://', ''),
    ('http://', ''),
    ('ftp://', ''),
    ('www.', 'http://'),
  ]) {
    if (lower.startsWith(prefix)) {
      var end = rest.length;
      for (var k = 0; k < rest.length; k++) {
        final c = rest[k];
        if (c == ' ' || c == '\t' || c == '\n' || c == '<') {
          end = k;
          break;
        }
      }
      end = _trimAutolinkEnd(rest.substring(0, end));
      final candidate = rest.substring(0, end);
      final afterScheme = implied.isEmpty
          ? candidate.substring(prefix.length)
          : candidate;
      final domain = afterScheme.split(RegExp(r'[/?#]')).first;
      if (!_validAutolinkDomain(domain)) return null;
      if (candidate.isEmpty) return null;
      return (candidate.length, '$implied$candidate');
    }
  }
  // Bare email.
  if (RegExp(r'^[A-Za-z0-9]').hasMatch(rest)) {
    final m = RegExp(r'^[A-Za-z0-9._+-]+@[A-Za-z0-9._-]+').firstMatch(rest);
    if (m != null) {
      var text = m.group(0)!;
      while (text.endsWith('.')) {
        text = text.substring(0, text.length - 1);
      }
      final at = text.indexOf('@');
      final domain = at >= 0 ? text.substring(at + 1) : '';
      if (at > 0 &&
          domain.contains('.') &&
          RegExp(r'[A-Za-z0-9]$').hasMatch(text)) {
        return (text.length, 'mailto:$text');
      }
    }
  }
  return null;
}

int _trimAutolinkEnd(String s) {
  var end = s.length;
  while (end > 0) {
    final kept = s.substring(0, end);
    final last = kept[kept.length - 1];
    if ('?!.,:*_~\'"'.contains(last)) {
      end -= 1;
      continue;
    }
    if (last == ')') {
      final opens = '('.allMatches(kept).length;
      final closes = ')'.allMatches(kept).length;
      if (closes > opens) {
        end -= 1;
        continue;
      }
      return end;
    }
    if (last == ';') {
      final amp = kept.lastIndexOf('&');
      if (amp >= 0) {
        final body = kept.substring(amp + 1, end - 1);
        if (body.isNotEmpty && RegExp(r'^[A-Za-z0-9]+$').hasMatch(body)) {
          end = amp;
          continue;
        }
      }
      return end;
    }
    return end;
  }
  return 0;
}

bool _validAutolinkDomain(String domain) {
  if (domain.isEmpty || !RegExp(r'^[A-Za-z0-9._-]+$').hasMatch(domain)) {
    return false;
  }
  final segments = domain.split('.');
  if (segments.length < 2 || segments.any((s) => s.isEmpty)) return false;
  final lastTwo = segments.length >= 2
      ? segments.sublist(segments.length - 2)
      : segments;
  return !lastTwo.any((s) => s.contains('_'));
}

/// Inline raw HTML at src[i] == '<': open tag, closing tag, comment,
/// processing instruction, declaration or CDATA. Returns the exclusive end
/// index, or -1. Mirrors the Rust engine's inline_html_end.
int inlineHtmlEnd(String src, int i) {
  if (i >= src.length || src[i] != '<') return -1;
  int findPat(int from, String pat) => src.indexOf(pat, from);
  // Comment (<!--> and <!---> count as empty comments).
  if (src.startsWith('!--', i + 1)) {
    if (src.startsWith('>', i + 4)) return i + 5;
    if (src.startsWith('->', i + 4)) return i + 6;
    final p = findPat(i + 4, '-->');
    return p < 0 ? -1 : p + 3;
  }
  if (src.startsWith('![CDATA[', i + 1)) {
    final p = findPat(i + 9, ']]>');
    return p < 0 ? -1 : p + 3;
  }
  if (i + 1 < src.length && src[i + 1] == '?') {
    final p = findPat(i + 2, '?>');
    return p < 0 ? -1 : p + 2;
  }
  bool isAlpha(String c) =>
      (c.codeUnitAt(0) | 0x20) >= 0x61 && (c.codeUnitAt(0) | 0x20) <= 0x7A;
  bool isAlnum(String c) =>
      isAlpha(c) || (c.codeUnitAt(0) >= 0x30 && c.codeUnitAt(0) <= 0x39);
  if (i + 1 < src.length && src[i + 1] == '!') {
    if (i + 2 >= src.length || !isAlpha(src[i + 2])) return -1;
    final p = findPat(i + 2, '>');
    return p < 0 ? -1 : p + 1;
  }
  var closing = false;
  var p = i + 1;
  if (p < src.length && src[p] == '/') {
    closing = true;
    p++;
  }
  if (p >= src.length || !isAlpha(src[p])) return -1;
  while (p < src.length && (isAlnum(src[p]) || src[p] == '-')) {
    p++;
  }
  int skipWs(int q) {
    while (q < src.length &&
        (src[q] == ' ' || src[q] == '\t' || src[q] == '\n')) {
      q++;
    }
    return q;
  }

  if (closing) {
    final q = skipWs(p);
    return (q < src.length && src[q] == '>') ? q + 1 : -1;
  }
  while (true) {
    final q = skipWs(p);
    if (q < src.length && src[q] == '>') return q + 1;
    if (q + 1 < src.length && src[q] == '/' && src[q + 1] == '>') return q + 2;
    if (q == p) return -1; // an attribute needs whitespace before it
    if (q >= src.length) return -1;
    final c0 = src[q];
    if (!(isAlpha(c0) || c0 == '_' || c0 == ':')) return -1;
    var r = q + 1;
    while (r < src.length &&
        (isAlnum(src[r]) ||
            src[r] == '_' ||
            src[r] == '.' ||
            src[r] == ':' ||
            src[r] == '-')) {
      r++;
    }
    final eq = skipWs(r);
    if (eq < src.length && src[eq] == '=') {
      final v = skipWs(eq + 1);
      if (v >= src.length) return -1;
      final quote = src[v];
      if (quote == '"' || quote == "'") {
        var w = v + 1;
        while (w < src.length && src[w] != quote) {
          w++;
        }
        if (w >= src.length) return -1;
        p = w + 1;
      } else {
        var w = v;
        while (w < src.length && !' \t\n"\'=<>`'.contains(src[w])) {
          w++;
        }
        if (w == v) return -1;
        p = w;
      }
    } else {
      p = r;
    }
  }
}

/// Curated named-entity table (the spec set plus common real-world names);
/// unknown entities stay literal, which is exactly the spec behavior.
const Map<String, String> _namedEntities = {
  'AElig': 'Æ',
  'Alpha': 'Α',
  'Beta': 'Β',
  'ClockwiseContourIntegral': '∲',
  'Dagger': '‡',
  'Dcaron': 'Ď',
  'Delta': 'Δ',
  'DifferentialD': 'ⅆ',
  'Gamma': 'Γ',
  'HilbertSpace': 'ℋ',
  'Lambda': 'Λ',
  'OElig': 'Œ',
  'Omega': 'Ω',
  'Phi': 'Φ',
  'Pi': 'Π',
  'Psi': 'Ψ',
  'Scaron': 'Š',
  'Sigma': 'Σ',
  'Theta': 'Θ',
  'Yuml': 'Ÿ',
  'aacute': 'á',
  'acirc': 'â',
  'acute': '´',
  'agrave': 'à',
  'alpha': 'α',
  'amp': '&',
  'apos': '\'',
  'aring': 'å',
  'asymp': '≈',
  'atilde': 'ã',
  'auml': 'ä',
  'beta': 'β',
  'brvbar': '¦',
  'bull': '•',
  'cap': '∩',
  'ccedil': 'ç',
  'cedil': '¸',
  'cent': '¢',
  'chi': 'χ',
  'circ': 'ˆ',
  'clubs': '♣',
  'copy': '©',
  'cup': '∪',
  'curren': '¤',
  'dagger': '†',
  'darr': '↓',
  'deg': '°',
  'delta': 'δ',
  'diams': '♦',
  'divide': '÷',
  'eacute': 'é',
  'ecirc': 'ê',
  'egrave': 'è',
  'empty': '∅',
  'emsp': ' ',
  'ensp': ' ',
  'epsilon': 'ε',
  'equiv': '≡',
  'eta': 'η',
  'euml': 'ë',
  'euro': '€',
  'exist': '∃',
  'fnof': 'ƒ',
  'forall': '∀',
  'frac12': '½',
  'frac14': '¼',
  'frac34': '¾',
  'frasl': '⁄',
  'gamma': 'γ',
  'ge': '≥',
  'gt': '>',
  'harr': '↔',
  'hearts': '♥',
  'hellip': '…',
  'iacute': 'í',
  'icirc': 'î',
  'iexcl': '¡',
  'infin': '∞',
  'int': '∫',
  'iota': 'ι',
  'iquest': '¿',
  'isin': '∈',
  'iuml': 'ï',
  'kappa': 'κ',
  'lambda': 'λ',
  'lang': '⟨',
  'laquo': '«',
  'larr': '←',
  'lceil': '⌈',
  'ldquo': '“',
  'le': '≤',
  'lfloor': '⌊',
  'loz': '◊',
  'lsquo': '‘',
  'lt': '<',
  'macr': '¯',
  'mdash': '—',
  'micro': 'µ',
  'middot': '·',
  'minus': '−',
  'mu': 'μ',
  'nbsp': ' ',
  'ndash': '–',
  'ne': '≠',
  'ngE': '≧̸',
  'not': '¬',
  'notin': '∉',
  'ntilde': 'ñ',
  'nu': 'ν',
  'oacute': 'ó',
  'ocirc': 'ô',
  'oelig': 'œ',
  'omega': 'ω',
  'oplus': '⊕',
  'ordf': 'ª',
  'ordm': 'º',
  'oslash': 'ø',
  'otilde': 'õ',
  'otimes': '⊗',
  'ouml': 'ö',
  'para': '¶',
  'permil': '‰',
  'perp': '⊥',
  'phi': 'φ',
  'pi': 'π',
  'plusmn': '±',
  'pound': '£',
  'prod': '∏',
  'psi': 'ψ',
  'quot': '"',
  'radic': '√',
  'rang': '⟩',
  'raquo': '»',
  'rarr': '→',
  'rceil': '⌉',
  'rdquo': '”',
  'reg': '®',
  'rfloor': '⌋',
  'rho': 'ρ',
  'rsquo': '’',
  'scaron': 'š',
  'sdot': '⋅',
  'sect': '§',
  'shy': '­',
  'sigma': 'σ',
  'spades': '♠',
  'sub': '⊂',
  'sum': '∑',
  'sup': '⊃',
  'sup1': '¹',
  'sup2': '²',
  'sup3': '³',
  'szlig': 'ß',
  'tau': 'τ',
  'theta': 'θ',
  'thinsp': ' ',
  'tilde': '˜',
  'times': '×',
  'trade': '™',
  'uacute': 'ú',
  'uarr': '↑',
  'ucirc': 'û',
  'uml': '¨',
  'upsilon': 'υ',
  'uuml': 'ü',
  'xi': 'ξ',
  'yen': '¥',
  'zeta': 'ζ',
  'zwj': '‍',
  'zwnj': '‌',
};

/// Parse an entity/numeric character reference starting at `&` (src[i]).
/// Returns (decoded string, chars consumed), or null.
(String, int)? parseEntity(String src, int i) {
  if (i >= src.length || src[i] != '&') return null;
  final semi = src.indexOf(';', i + 1);
  if (semi < 0 || semi - i > 33) return null;
  final body = src.substring(i + 1, semi);
  if (body.startsWith('#')) {
    var digits = body.substring(1);
    var radix = 10;
    if (digits.startsWith('x') || digits.startsWith('X')) {
      digits = digits.substring(1);
      radix = 16;
    }
    if (digits.isEmpty || digits.length > 7) return null;
    final n = int.tryParse(digits, radix: radix);
    if (n == null) return null;
    final code = (n == 0 || n > 0x10FFFF || (n >= 0xD800 && n <= 0xDFFF))
        ? 0xFFFD
        : n;
    return (String.fromCharCode(code), semi - i + 1);
  }
  final v = _namedEntities[body];
  return v == null ? null : (v, semi - i + 1);
}

/// Unescape backslash-escaped ASCII punctuation (public mirror).
String unescapeMd(String s) => _unescapeMd(s);

/// Unescape backslash-escaped ASCII punctuation.
String _unescapeMd(String s) {
  final out = StringBuffer();
  var i = 0;
  while (i < s.length) {
    if (s[i] == r'\' && i + 1 < s.length && _asciiPunct.contains(s[i + 1])) {
      out.write(s[i + 1]);
      i += 2;
    } else if (s[i] == '&') {
      // Entity references decode inside destinations/titles/info strings.
      final ent = parseEntity(s, i);
      if (ent != null) {
        out.write(ent.$1);
        i += ent.$2;
      } else {
        out.write(s[i]);
        i++;
      }
    } else {
      out.write(s[i]);
      i++;
    }
  }
  return out.toString();
}

/// Inline-link suffix `(dest "title")` parsed right AFTER the `(`. Returns
/// dest/title and the index just past the closing `)`. Mirrors the Rust
/// engine.
({String dest, String? title, int next})? parseLinkSuffix(String src, int i) {
  final n = src.length;
  bool ws(String c) => c == ' ' || c == '\t' || c == '\n';
  while (i < n && ws(src[i])) {
    i++;
  }
  String dest;
  if (i < n && src[i] == '<') {
    var j = i + 1;
    while (j < n && src[j] != '>' && src[j] != '\n' && src[j] != '<') {
      j++;
    }
    if (j >= n || src[j] != '>') return null;
    dest = _unescapeMd(src.substring(i + 1, j));
    i = j + 1;
  } else {
    var depth = 0;
    final start = i;
    while (i < n) {
      final c = src[i];
      if (ws(c)) break;
      if (c == r'\' && i + 1 < n) {
        i += 2;
        continue;
      }
      if (c == '(') depth++;
      if (c == ')') {
        if (depth == 0) break;
        depth--;
      }
      i++;
    }
    if (depth != 0) return null;
    dest = _unescapeMd(src.substring(start, i));
  }
  while (i < n && ws(src[i])) {
    i++;
  }
  String? title;
  if (i < n && (src[i] == '"' || src[i] == "'" || src[i] == '(')) {
    final close = src[i] == '(' ? ')' : src[i];
    var j = i + 1;
    final start = j;
    while (j < n && src[j] != close) {
      // A backslash escapes the next char (so an escaped close delimiter does
      // not end the title); keep the RAW span and decode it once at the end —
      // exactly as Rust `parse_link_suffix` does.
      if (src[j] == r'\' && j + 1 < n) {
        j += 2;
        continue;
      }
      j++;
    }
    if (j >= n) return null;
    // Decode through `_unescapeMd`, which (mirroring Rust `unescape_md`) only
    // unescapes backslash before ASCII PUNCTUATION and decodes entities.
    // Building the string inline here instead dropped the backslash before ANY
    // character, so `[a](/u "C:\name")` yielded the title `C:name` — silent
    // data loss, and a Rust/Dart divergence. (P1-2 confirmed drift.)
    title = _unescapeMd(src.substring(start, j));
    i = j + 1;
    while (i < n && ws(src[i])) {
      i++;
    }
  }
  if (i < n && src[i] == ')') {
    return (dest: dest, title: title, next: i + 1);
  }
  return null;
}

/// CommonMark autolink target for `<inner>`: absolute URI → itself, bare
/// email → mailto:, else null. Mirrors the Rust engine.
String? autolinkTarget(String inner) {
  if (inner.isEmpty || inner.contains(RegExp(r'[\s<]'))) return null;
  if (RegExp(r'^[A-Za-z][A-Za-z0-9+.-]{1,31}:.').hasMatch(inner)) return inner;
  if (!inner.contains(r'\') &&
      RegExp(r'^[^@\s]+@[^@\s]+\.[^@\s.]+$').hasMatch(inner)) {
    return 'mailto:$inner';
  }
  return null;
}

/// Parse inline Markdown (`**b**`, `*i*`/`_i_`, `` `c` ``, `~~s~~`,
/// `[t](url)`, `\*escapes\*`, `<autolinks>`) into clean text plus marks.
({String text, List<Mark> marks}) parseInline(
  String src, {
  Map<String, ({String dest, String? title})> defs = const {},
}) {
  final out = StringBuffer();
  final marks = <Mark>[];
  final delims = <_Delim>[];
  var i = 0;

  void addInner(String inner, String type, {String? href}) {
    final start = out.length;
    final parsed = parseInline(inner, defs: defs);
    out.write(parsed.text);
    for (final m in parsed.marks) {
      marks.add(m.shifted(start));
    }
    marks.add(Mark(start, out.length, type, href: href));
  }

  void addLink(String kind, String label, String href, String? title) {
    final start = out.length;
    final parsed = parseInline(label, defs: defs);
    out.write(parsed.text);
    for (final m in parsed.marks) {
      marks.add(m.shifted(start));
    }
    marks.add(Mark(start, out.length, kind, href: href, title: title));
  }

  while (i < src.length) {
    // Inline math, LaTeX form: \( ... \) — checked before the escape arm,
    // which would otherwise eat the `\(`.
    if (src[i] == r'\' && i + 1 < src.length && src[i + 1] == '(') {
      final close = src.indexOf(r'\)', i + 2);
      if (close > i) {
        final inner = src.substring(i + 2, close).trim();
        if (inner.isNotEmpty) {
          final start = out.length;
          out.write(inner);
          marks.add(Mark(start, out.length, 'math'));
          i = close + 2;
          continue;
        }
      }
    }
    // Inline math, dollar form (Pandoc rules: opener not followed by
    // whitespace, closer not preceded by whitespace or followed by a digit).
    if (src[i] == kDollar && (i + 1 >= src.length || src[i + 1] != kDollar)) {
      final close = _findMathCloser(src, i + 1);
      if (close > 0) {
        final inner = src.substring(i + 1, close);
        final start = out.length;
        out.write(inner);
        marks.add(Mark(start, out.length, 'math'));
        i = close + 1;
        continue;
      }
    }
    // Backslash escape: `\*` is a literal `*` (any ASCII punctuation).
    if (src[i] == r'\' &&
        i + 1 < src.length &&
        _asciiPunct.contains(src[i + 1])) {
      out.write(src[i + 1]);
      i += 2;
      continue;
    }
    // Entity / numeric character reference: decodes to plain TEXT (the
    // result can't open emphasis or any structure).
    if (src[i] == '&') {
      final ent = parseEntity(src, i);
      if (ent != null) {
        out.write(ent.$1);
        i += ent.$2;
        continue;
      }
    }
    // Autolink: <https://…> / <user@host> — the URL is both text and target.
    if (src[i] == '<') {
      final close = src.indexOf('>', i + 1);
      if (close > i) {
        final inner = src.substring(i + 1, close);
        final href = autolinkTarget(inner);
        if (href != null) {
          final start = out.length;
          out.write(inner);
          marks.add(Mark(start, out.length, 'link', href: href));
          i = close + 1;
          continue;
        }
      }
      // Raw inline HTML: a valid tag/comment/PI/declaration/CDATA shape
      // passes through verbatim under an `html` mark.
      final htmlEnd = inlineHtmlEnd(src, i);
      if (htmlEnd > i) {
        final raw = src.substring(i, htmlEnd);
        final start = out.length;
        out.write(raw);
        marks.add(Mark(start, out.length, 'html'));
        i = htmlEnd;
        continue;
      }
    }
    // Image: ![alt](dest "title") | ![alt][label] | ![alt][] | ![alt] —
    // same bridge as links; the alt keeps its inner marks (HTML flattens
    // them to plain text, markdown re-renders them).
    if (src[i] == '!' && i + 1 < src.length && src[i + 1] == '[') {
      final close = matchingBracket(src, i + 1);
      if (close > i + 2) {
        final label = src.substring(i + 2, close);
        if (close + 1 < src.length && src[close + 1] == '(') {
          final suffix = parseLinkSuffix(src, close + 2);
          if (suffix != null) {
            addLink('image', label, suffix.dest, suffix.title);
            i = suffix.next;
            continue;
          }
        }
        if (defs.isNotEmpty) {
          var refLabel = label;
          var next = close + 1;
          if (close + 1 < src.length && src[close + 1] == '[') {
            final end2 = matchingBracket(src, close + 1);
            if (end2 > close + 1) {
              final second = src.substring(close + 2, end2);
              if (second.isNotEmpty) refLabel = second;
              next = end2 + 1;
            }
          }
          final def = defs[normalizeLabel(refLabel)];
          if (def != null) {
            addLink('image', label, def.dest, def.title);
            i = next;
            continue;
          }
        }
      }
    }
    // Footnote reference: `[^label]` (GFM). The brackets and caret strip to
    // the bare label under a `footnote` mark carrying the label as href —
    // the same delimiter-strip + mark shape inline math uses. Checked before
    // the link arm so `[^x]` never parses as a shortcut link. Mirrors Rust.
    if (src[i] == '[' && i + 1 < src.length && src[i + 1] == '^') {
      final close = matchingBracket(src, i);
      if (close > i + 2) {
        final label = src.substring(i + 2, close);
        if (!label.contains(RegExp(r'[\s\[\]^]'))) {
          final start = out.length;
          out.write(label);
          marks.add(Mark(start, out.length, 'footnote', href: label));
          i = close + 1;
          continue;
        }
      }
    }
    // Links: [text](dest "title") | [text][label] | [text][] | [shortcut]
    if (src[i] == '[') {
      final close = matchingBracket(src, i);
      // `close > i` (not `> i + 1`): an EMPTY label is legal for the inline form
      // — `[](/url)` is a valid CommonMark link with empty text, and the Rust
      // engine accepts it (`matching_bracket` alone gates its branch). Requiring
      // a non-empty label made Dart leave `[](/url)` literal while Rust produced
      // a zero-width anchor, so `x [](/url) y` differed in BOTH text and marks
      // between server import and client paste. The reference forms below still
      // require a non-empty label, matching Rust's `!label.is_empty()`.
      // (-1 = no match, so `> i` also rejects that.)
      if (close > i) {
        final label = src.substring(i + 1, close);
        // CommonMark §6.3: a link's text may not itself contain a link. When
        // the label already holds one, the OUTER brackets stay literal and the
        // inner link wins — `[foo [bar](/uri)](/baz)` is `[foo <a>bar</a>](/baz)`,
        // not a nested anchor. Images are fine (`[![alt](img)](url)` is a valid
        // linked image), so only `link` marks disqualify. Mirrors Rust
        // `label_contains_link`; without it the Dart mirror produced a different
        // mark range than the engine. (P1-2 confirmed drift.)
        // Evaluated LAZILY and memoized, mirroring the Rust engine. The flag only
        // gates the two link forms below, so deferring it to the point a form
        // actually matches is output-identical (differential-fuzzed against the
        // eager order, ~2.8M inputs, zero divergence).
        //
        // SCOPE OF THE WIN — do not overstate it: it removes the cost only for
        // brackets matching NO link form (`[[[[a]]]]` at depth 24: ~12s -> 0ms).
        // It does NOT help when every level IS a link — `[[[a](/u)](/u)](/u)`
        // stays exponential, because each level forces the check, is rejected,
        // and the span is re-parsed on the way back down. This runs on the PASTE
        // path, so that residual is a potential UI freeze on crafted input. It is
        // PRE-EXISTING (eager was worse) — see docs/code-review-2026-07-20.md.
        bool? nestedCache;
        bool labelHasLink() => nestedCache ??=
            parseInline(label, defs: defs).marks.any((m) => m.type == 'link');

        if (close + 1 < src.length && src[close + 1] == '(') {
          final suffix = parseLinkSuffix(src, close + 2);
          if (suffix != null && !labelHasLink()) {
            addLink('link', label, suffix.dest, suffix.title);
            i = suffix.next;
            continue;
          }
        }
        // Reference forms still require a NON-empty label (Rust:
        // `!label.is_empty()`); only the inline form above accepts `[](/url)`.
        if (label.isNotEmpty && defs.isNotEmpty) {
          var refLabel = label;
          var next = close + 1;
          if (close + 1 < src.length && src[close + 1] == '[') {
            final end2 = matchingBracket(src, close + 1);
            if (end2 > close + 1) {
              final second = src.substring(close + 2, end2);
              if (second.isNotEmpty) refLabel = second;
              next = end2 + 1;
            }
          }
          final def = defs[normalizeLabel(refLabel)];
          if (def != null && !labelHasLink()) {
            addLink('link', label, def.dest, def.title);
            i = next;
            continue;
          }
        }
      }
    }
    // Emphasis delimiters (* and _): record the run with flanking; pairing
    // happens after the scan (spec algorithm, mirrors the Rust engine).
    if (src[i] == '*' || src[i] == '_') {
      final c = src[i];
      var j = i;
      while (j < src.length && src[j] == c) {
        j++;
      }
      final count = j - i;
      final f = _flanking(
        c,
        i == 0 ? null : src[i - 1],
        j < src.length ? src[j] : null,
      );
      delims.add(_Delim(c, out.length, count, f.open, f.close));
      out.write(c * count);
      i = j;
      continue;
    }
    // GFM extended autolinks: bare www./http(s)/ftp URLs and emails at a
    // word boundary (start, whitespace, or `*`/`_`/`~`/`(`).
    if (i == 0 ||
        src[i - 1] == ' ' ||
        src[i - 1] == '\t' ||
        src[i - 1] == '\n' ||
        '*_~('.contains(src[i - 1])) {
      final auto = extendedAutolink(src, i);
      if (auto != null) {
        final text = src.substring(i, i + auto.$1);
        final start = out.length;
        out.write(text);
        marks.add(Mark(start, out.length, 'link', href: auto.$2));
        i += auto.$1;
        continue;
      }
    }
    // GFM strikethrough: one or two tildes close on a run of the same
    // length (three or more stay literal).
    if (src[i] == '~') {
      var n = 0;
      while (i + n < src.length && src[i + n] == '~') {
        n++;
      }
      if (n <= 2) {
        var j = i + n;
        var close = -1;
        while (j < src.length) {
          if (src[j] == '~' && (j == 0 || src[j - 1] != r'\')) {
            var m = 0;
            while (j + m < src.length && src[j + m] == '~') {
              m++;
            }
            if (m == n && j > i + n) {
              close = j;
              break;
            }
            j += m;
          } else {
            j++;
          }
        }
        if (close >= 0) {
          addInner(src.substring(i + n, close), 'strike');
          i = close + n;
          continue;
        }
      }
      out.write('~' * n);
      i += n;
      continue;
    }
    // Inline code: an N-backtick run closes only on a run of exactly N;
    // line endings become spaces; one leading+trailing space strips when
    // both exist and the content isn't all spaces. No escapes inside.
    if (src[i] == '`') {
      var n = 0;
      while (i + n < src.length && src[i + n] == '`') {
        n++;
      }
      var j = i + n;
      var close = -1;
      while (j < src.length) {
        if (src[j] == '`') {
          var m = 0;
          while (j + m < src.length && src[j + m] == '`') {
            m++;
          }
          if (m == n) {
            close = j;
            break;
          }
          j += m;
        } else {
          j++;
        }
      }
      if (close >= 0) {
        var inner = src.substring(i + n, close).replaceAll('\n', ' ');
        if (inner.length >= 2 &&
            inner.startsWith(' ') &&
            inner.endsWith(' ') &&
            inner.trim().isNotEmpty) {
          inner = inner.substring(1, inner.length - 1);
        }
        final start = out.length;
        out.write(inner);
        marks.add(Mark(start, out.length, 'code'));
        i = close + n;
        continue;
      }
      // No closer: the run is literal text.
      out.write('`' * n);
      i += n;
      continue;
    }
    out.write(src[i]);
    i++;
  }
  final processed = _processEmphasis(out.toString(), marks, delims);
  // NOTE: parse output is intentionally NOT merge-normalized — nested
  // same-type emphasis (`<em><em>`) must survive for spec fidelity; the
  // editor's own operations (applyMark etc.) still normalize.
  marks.sort((a, b) => a.start != b.start ? a.start - b.start : a.end - b.end);
  return (text: processed, marks: marks);
}

/// One run of `*`/`_` delimiters, tracked in output coordinates.
class _Delim {
  _Delim(this.c, this.start, this.count, this.canOpen, this.canClose)
    : curStart = start,
      orig = count;
  final String c;
  final int start;
  int curStart;
  int count;
  final int orig;
  final bool canOpen;
  final bool canClose;
}

bool _isMdPunct(String? c) {
  if (c == null) return false;
  if (_asciiPunct.contains(c)) return true;
  // CommonMark 0.31: "Unicode punctuation" = categories P* AND S* (symbols
  // — currency, math, arrows). Mirrors the Rust engine's block coverage so
  // the editor's emphasis matches the exporter's (`*€*x` is NOT <em>).
  final u = c.codeUnitAt(0);
  return (u >= 0x00A1 && u <= 0x00A9) ||
      (u >= 0x00AB && u <= 0x00B1) ||
      u == 0x00B4 ||
      (u >= 0x00B6 && u <= 0x00B8) ||
      u == 0x00BB ||
      u == 0x00BF ||
      u == 0x00D7 ||
      u == 0x00F7 ||
      (u >= 0x2000 && u <= 0x206F) ||
      (u >= 0x20A0 && u <= 0x20CF) ||
      (u >= 0x2100 && u <= 0x2BFF) ||
      (u >= 0x3000 && u <= 0x303F) ||
      (u >= 0xFE30 && u <= 0xFE4F) ||
      (u >= 0xFF01 && u <= 0xFF0F) ||
      (u >= 0xFF1A && u <= 0xFF20) ||
      (u >= 0xFF3B && u <= 0xFF40) ||
      (u >= 0xFF5B && u <= 0xFF65);
}

/// A CJK character (BMP): ideographs, kana, hangul, bopomofo, AND CJK
/// punctuation (`。、！？「」…`). Used to make emphasis CJK-friendly — see
/// [_flanking]. Kept byte-identical to the Rust `is_cjk` (crates/markdown).
bool _isCjk(String? c) {
  if (c == null || c.isEmpty) return false;
  final u = c.codeUnitAt(0);
  return (u >= 0x1100 && u <= 0x11FF) || // Hangul Jamo
      (u >= 0x2E80 && u <= 0x2EFF) || //    CJK Radicals Supplement
      (u >= 0x3000 && u <= 0x303F) || //    CJK Symbols and Punctuation
      (u >= 0x3040 && u <= 0x30FF) || //    Hiragana + Katakana
      (u >= 0x3100 && u <= 0x312F) || //    Bopomofo
      (u >= 0x3130 && u <= 0x318F) || //    Hangul Compatibility Jamo
      (u >= 0x31C0 && u <= 0x31EF) || //    CJK Strokes
      (u >= 0x3200 && u <= 0x33FF) || //    Enclosed CJK + CJK Compatibility
      (u >= 0x3400 && u <= 0x4DBF) || //    CJK Ext A
      (u >= 0x4E00 && u <= 0x9FFF) || //    CJK Unified Ideographs
      (u >= 0xA000 && u <= 0xA4CF) || //    Yi
      (u >= 0xAC00 && u <= 0xD7AF) || //    Hangul Syllables
      (u >= 0xF900 && u <= 0xFAFF) || //    CJK Compatibility Ideographs
      (u >= 0xFE30 && u <= 0xFE4F) || //    CJK Compatibility Forms
      (u >= 0xFF00 && u <= 0xFFEF); //      Halfwidth and Fullwidth Forms
}

// CJK-friendly emphasis (markdown-cjk-friendly amendment). Plain CommonMark
// flanking treats CJK punctuation (`。`) as "punctuation", so `**加粗。**后文`
// can't close — the `。` before `**` and a letter after fail the flanking test.
// A Chinese sentence ends in `。`/`,` far more often than in a space, so this
// bit constantly (and broke mica's OWN round trip: it exports `**…。**x` and
// then couldn't re-parse it). Fix: split "punctuation" into NON-CJK punctuation
// (keeps the strict rule) and CJK punctuation/characters (which instead RELAX
// flanking the way whitespace does in Latin — a CJK char is a word boundary).
// ASCII inputs are unaffected (`_isCjk` is false), so the CommonMark scoreboard
// stays 641/641. Mirrors the Rust `flanking` in crates/markdown.
({bool open, bool close}) _flanking(String c, String? prev, String? next) {
  final prevWs = prev == null || prev.trim().isEmpty;
  final nextWs = next == null || next.trim().isEmpty;
  final prevCjk = _isCjk(prev);
  final nextCjk = _isCjk(next);
  final prevNcp = _isMdPunct(prev) && !prevCjk; // non-CJK punctuation
  final nextNcp = _isMdPunct(next) && !nextCjk;
  final left = !nextWs && (!nextNcp || prevWs || prevNcp || prevCjk);
  final right = !prevWs && (!prevNcp || nextWs || nextNcp || nextCjk);
  if (c == '_') {
    return (
      open: left && (!right || prevNcp || prevCjk),
      close: right && (!left || nextNcp || nextCjk),
    );
  }
  return (open: left, close: right);
}

/// Spec process-emphasis: pair closers with the nearest valid opener (same
/// char, rule of 3), strong before em; delete used delimiter characters and
/// remap mark offsets. Returns the rebuilt text.
String _processEmphasis(String text, List<Mark> marks, List<_Delim> delims) {
  if (delims.isEmpty) return text;
  final deletions = <(int, int)>[];
  final pending = <Mark>[];

  var closerI = 0;
  while (closerI < delims.length) {
    final cl = delims[closerI];
    if (cl.count == 0 || !cl.canClose) {
      closerI++;
      continue;
    }
    int? openerI;
    for (var k = closerI - 1; k >= 0; k--) {
      final o = delims[k];
      if (o.count == 0 || !o.canOpen || o.c != cl.c) continue;
      if ((o.canClose || cl.canOpen) &&
          (o.orig + cl.orig) % 3 == 0 &&
          !(o.orig % 3 == 0 && cl.orig % 3 == 0)) {
        continue;
      }
      openerI = k;
      break;
    }
    if (openerI == null) {
      closerI++;
      continue;
    }
    final o = delims[openerI];
    final useN = (o.count >= 2 && cl.count >= 2) ? 2 : 1;
    o.count -= useN;
    final oDel = o.start + o.count;
    deletions.add((oDel, useN));
    final cDel = cl.curStart;
    deletions.add((cDel, useN));
    cl.curStart += useN;
    cl.count -= useN;
    pending.add(Mark(oDel + useN, cDel, useN == 2 ? 'bold' : 'italic'));
    for (var k = openerI + 1; k < closerI; k++) {
      delims[k].count = 0;
    }
    if (cl.count == 0) closerI++;
  }

  if (deletions.isEmpty) return text;
  marks.addAll(pending);
  deletions.sort((a, b) => a.$1.compareTo(b.$1));

  final keep = StringBuffer();
  final removedBefore = List<int>.filled(text.length + 1, 0);
  var di = 0;
  var removed = 0;
  var skipUntil = 0;
  for (var idx = 0; idx < text.length; idx++) {
    removedBefore[idx] = removed;
    if (idx >= skipUntil && di < deletions.length && deletions[di].$1 == idx) {
      skipUntil = idx + deletions[di].$2;
      di++;
    }
    if (idx < skipUntil) {
      removed++;
    } else {
      keep.write(text[idx]);
    }
  }
  removedBefore[text.length] = removed;

  for (var k = 0; k < marks.length; k++) {
    final m = marks[k];
    final ns = m.start - removedBefore[m.start.clamp(0, text.length)];
    final ne = m.end - removedBefore[m.end.clamp(0, text.length)];
    marks[k] = Mark(ns, ne, m.type, href: m.href, title: m.title);
  }
  marks.removeWhere((m) => m.end <= m.start);
  return keep.toString();
}

/// A paragraph whose text LOOKS like a block marker (`- x`, `> x`, `# x`,
/// `1. x`, `---`, `===`) must escape its leader or it changes kind on re-parse.
///
/// Faithfully mirrors the Rust engine (crates/markdown/src/lib.rs
/// `escape_block_leader`). It previously did NOT — its doc-comment claimed to
/// mirror Rust while (a) not stripping spaces before the divider check and
/// (b) omitting setext underlines entirely. Result: a paragraph line `===` (or
/// `--`, `-- -`) exported unescaped and, on re-import, turned the PRECEDING
/// paragraph into a setext heading — silent round-trip data loss.
/// (docs/code-review-2026-07-20.md P1-1.)
String escapeBlockLeader(String line) {
  // Divider: strip spaces first (`-- -` is a thematic break), then >=3 dashes.
  final compact = line.replaceAll(' ', '');
  final dividerLike =
      compact.length >= 3 && compact.split('').every((c) => c == '-');
  // Setext underline: a whole line of `=` or of `-` underlines the paragraph
  // above it as a heading on re-parse.
  final setextLike = line.isNotEmpty &&
      (line.split('').every((c) => c == '=') ||
          line.split('').every((c) => c == '-'));
  // ATX: `#`+ then a space (any count, matching Rust — not just 1–6).
  final atxLike =
      line.startsWith('#') && line.replaceFirst(RegExp(r'^#+'), '').startsWith(' ');
  if (line.startsWith('- ') ||
      line.startsWith('+ ') ||
      line.startsWith('> ') ||
      dividerLike ||
      setextLike ||
      atxLike) {
    return '\\$line';
  }
  final numbered = RegExp(r'^(\d+)\. ').firstMatch(line);
  if (numbered != null) {
    return '${numbered.group(1)}\\. ${line.substring(numbered.end)}';
  }
  return line;
}

/// Escape characters the inline grammar would otherwise interpret.
String escapeInline(String text) {
  final out = StringBuffer();
  for (var i = 0; i < text.length; i++) {
    final c = text[i];
    // A backslash right before a newline IS a hard break — keep it raw.
    if (c == r'\' && i + 1 < text.length && text[i + 1] == '\n') {
      out.write(c);
      continue;
    }
    if (r'\*_`~[]<'.contains(c) || c == kDollar) out.write(r'\');
    out.write(c);
  }
  return out.toString();
}

/// Serialize text + marks back to inline Markdown (copy/export). Properly
/// NESTED rendering — the outermost mark opens once over its whole range,
/// inner marks recurse inside (`**bold *italic* tail**`); literal segments
/// are escaped; code spans stay raw; a link whose text is its own target
/// becomes an `<autolink>`. Mirrors the Rust engine's render_span.
String inlineToMarkdown(String text, List<Mark> marks) {
  if (marks.isEmpty) return text;
  return _renderSpan(text, 0, text.length, marks);
}

String _renderSpan(String text, int lo, int hi, List<Mark> marks) {
  final out = StringBuffer();
  var pos = lo;
  while (pos < hi) {
    // Next mark by clipped start; ties prefer the widest (outermost).
    Mark? pick;
    var ps = 0, pe = 0;
    for (final m in marks) {
      final s = m.start < pos ? pos : m.start;
      final e = m.end > hi ? hi : m.end;
      if (e <= s) continue;
      if (pick == null || s < ps || (s == ps && e > pe)) {
        pick = m;
        ps = s;
        pe = e;
      }
    }
    if (pick == null) {
      out.write(escapeInline(text.substring(pos, hi)));
      break;
    }
    var lead = escapeInline(text.substring(pos, ps));
    if (pick.type == 'link' && lead.endsWith('!')) {
      lead = '${lead.substring(0, lead.length - 1)}\\!';
    }
    out.write(lead);
    if (pick.type == 'html') {
      // Raw inline HTML writes back verbatim.
      out.write(text.substring(ps, pe));
      pos = pe;
      continue;
    }
    if (pick.type == 'math') {
      // LaTeX source is literal — canonical dollar form.
      out.write(kDollar);
      out.write(text.substring(ps, pe));
      out.write(kDollar);
      pos = pe;
      continue;
    }
    if (pick.type == 'footnote') {
      // The span text IS the label; the `[^…]` syntax is restored from the
      // mark's href (the label survives even if the span text was edited).
      final label = pick.href ?? text.substring(ps, pe);
      out.write('[^$label]');
      pos = pe;
      continue;
    }
    if (pick.type == 'code') {
      // Code spans are literal — no escaping, no nested marks. The fence is
      // one backtick longer than any run inside; a space pads content that
      // starts/ends with a backtick or with stripped-on-read spaces.
      final raw = text.substring(ps, pe);
      var longest = 0, cur = 0;
      for (var k = 0; k < raw.length; k++) {
        if (raw[k] == '`') {
          cur++;
          if (cur > longest) longest = cur;
        } else {
          cur = 0;
        }
      }
      final fence = '`' * (longest + 1);
      final pad =
          raw.startsWith('`') ||
          raw.endsWith('`') ||
          (raw.startsWith(' ') && raw.endsWith(' ') && raw.trim().isNotEmpty);
      out.write(pad ? '$fence $raw $fence' : '$fence$raw$fence');
      pos = pe;
      continue;
    }
    final inner = [
      for (final m in marks)
        if (!identical(m, pick) && m.end > ps && m.start < pe) m,
    ];
    final body = _renderSpan(text, ps, pe, inner);
    switch (pick.type) {
      case 'bold':
        out.write('**$body**');
      case 'italic':
        out.write('*$body*');
      case 'strike':
        out.write('~~$body~~');
      case 'link':
        final href = pick.href ?? '';
        final plain = text.substring(ps, pe);
        if (pick.title == null &&
            plain.startsWith('www.') &&
            href == 'http://$plain') {
          // A bare `www.` link writes back bare (GFM re-links it on read).
          out.write(plain);
        } else if (pick.title == null &&
            (href == plain || href == 'mailto:$plain')) {
          out.write('<$plain>');
        } else {
          final dest = href.contains(RegExp(r'\s')) ? '<$href>' : href;
          final t = pick.title;
          out.write(
            t == null
                ? '[$body]($dest)'
                : '[$body]($dest "${t.replaceAll('"', r'\"')}")',
          );
        }
      case 'image':
        final href = pick.href ?? '';
        final dest = href.contains(RegExp(r'\s')) ? '<$href>' : href;
        final t = pick.title;
        out.write(
          t == null
              ? '![$body]($dest)'
              : '![$body]($dest "${t.replaceAll('"', r'\"')}")',
        );
      default:
        out.write(body);
    }
    pos = pe;
  }
  return out.toString();
}

/// Entity-escape text for HTML body content.
String escapeHtml(String s) =>
    s.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');

/// Entity-escape text for a double-quoted HTML attribute value.
String escapeHtmlAttr(String s) => escapeHtml(s).replaceAll('"', '&quot;');

/// The inline text + [marks] rendered as HTML — the rich flavor written to the
/// clipboard so plain editors read stripped `text/plain` while Markdown editors
/// (Typora, Obsidian) read `text/html` and keep the formatting. Mirrors
/// [inlineToMarkdown]'s outermost-first recursion so nested emphasis nests; HTML
/// needs only entity-escaping (no delimiter fencing).
String inlineToHtml(String text, List<Mark> marks) =>
    _renderSpanHtml(text, 0, text.length, marks);

String _renderSpanHtml(String text, int lo, int hi, List<Mark> marks) {
  final out = StringBuffer();
  var pos = lo;
  while (pos < hi) {
    Mark? pick;
    var ps = 0, pe = 0;
    for (final m in marks) {
      final s = m.start < pos ? pos : m.start;
      final e = m.end > hi ? hi : m.end;
      if (e <= s) continue;
      if (pick == null || s < ps || (s == ps && e > pe)) {
        pick = m;
        ps = s;
        pe = e;
      }
    }
    if (pick == null) {
      out.write(escapeHtml(text.substring(pos, hi)));
      break;
    }
    out.write(escapeHtml(text.substring(pos, ps)));
    // Literal-content leaves (no nested marks).
    if (pick.type == 'code') {
      out.write('<code>${escapeHtml(text.substring(ps, pe))}</code>');
      pos = pe;
      continue;
    }
    if (pick.type == 'math') {
      // The LaTeX source keeps its `$` delimiters so Markdown editors reading
      // text/html see the formula; the data-mica-math wrapper lets our own
      // HTML→Markdown converter pass it through verbatim (no escaping) so a
      // mica→mica round trip re-parses the math mark instead of dropping it.
      out.write(
        '<span data-mica-math="1">\$'
        '${escapeHtml(text.substring(ps, pe))}\$</span>',
      );
      pos = pe;
      continue;
    }
    if (pick.type == 'footnote') {
      // Mirror inlineToMarkdown: the label lives on the mark's href (the span
      // text may have been edited). External readers see a superscript label;
      // our converter restores `[^label]` from the attribute.
      final label = pick.href ?? text.substring(ps, pe);
      out.write(
        '<sup data-mica-footnote="${escapeHtmlAttr(label)}">'
        '${escapeHtml(text.substring(ps, pe))}</sup>',
      );
      pos = pe;
      continue;
    }
    if (pick.type == 'html') {
      out.write(escapeHtml(text.substring(ps, pe)));
      pos = pe;
      continue;
    }
    final inner = [
      for (final m in marks)
        if (!identical(m, pick) && m.end > ps && m.start < pe) m,
    ];
    final body = _renderSpanHtml(text, ps, pe, inner);
    switch (pick.type) {
      case 'bold':
        out.write('<strong>$body</strong>');
      case 'italic':
        out.write('<em>$body</em>');
      case 'strike':
        out.write('<s>$body</s>');
      case 'link':
        out.write('<a href="${escapeHtmlAttr(pick.href ?? '')}">$body</a>');
      case 'image':
        out.write(
          '<img src="${escapeHtmlAttr(pick.href ?? '')}" '
          'alt="${escapeHtmlAttr(text.substring(ps, pe))}">',
        );
      default:
        out.write(body);
    }
    pos = pe;
  }
  return out.toString();
}
