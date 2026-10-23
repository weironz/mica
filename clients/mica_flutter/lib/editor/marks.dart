import 'package:flutter/widgets.dart';
import 'model.dart' show kMonoFont;
import '../ui/theme_tokens.dart';

/// Inline rich-text marks stored additively over a block's plain text (in
/// `data.marks`). The text stays clean; a mark is a `[start, end)` range with a
/// type (and an href for links). See docs/architecture/editor-engine.md.
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
      // Zero-width is legal for a LINK: `[](/url)` is an empty-text link the
      // Rust engine keeps; dropping it here erased it on the next save.
      if (start != null &&
          end != null &&
          type != null &&
          (end > start || (type == 'link' && end == start))) {
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

/// Build a styled [TextSpan] for [text] with [marks] applied over [base].
///
/// [tokens] is the palette. It has to be a parameter: this is a plain top-level
/// function with no widget tree to read from, and its five colours all have to
/// differ between light and dark.
TextSpan buildMarkedSpan(
  String text,
  List<Mark> marks,
  TextStyle base,
  MicaTokens tokens,
) {
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
          style = style.copyWith(
            fontFamily: kMonoFont,
            color: tokens.editor.inlineCodeInk,
          );
        case 'strike':
          decorations.add(TextDecoration.lineThrough);
        case 'link':
          style = style.copyWith(color: tokens.accent.primary);
          decorations.add(TextDecoration.underline);
        case 'math':
          // LaTeX source shown styled until real typesetting lands.
          style = style.copyWith(
            fontFamily: kMonoFont,
            fontStyle: FontStyle.italic,
            // The same violet the highlighter paints keywords with: LaTeX shown
            // as source IS syntax, and two violets a shade apart would just be
            // two things nobody can keep in sync.
            color: tokens.code.keyword,
            backgroundColor: tokens.code.keyword.withValues(alpha: 0.08),
          );
        case 'footnote':
          // GFM reference chip: small superscript label in link blue. The
          // raised baseline + reduced size reads as a footnote marker.
          style = style.copyWith(
            color: tokens.accent.primary,
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
String normalizeLabel(String label) => label
    .trim()
    .split(RegExp(r'\s+'))
    .join(' ')
    .toLowerCase()
    .replaceAll('ß', 'ss');

/// One past the code span / autolink / raw-inline-HTML region opening at
/// [i], or -1 when [i] opens none. Mirrors Rust `inline_span_end`.
///
/// These three bind tighter than every other inline construct (CommonMark
/// §6.1), so scanners looking for a delimiter must step over them whole.
int _inlineSpanEnd(String src, int i) {
  final c = src[i];
  if (c == '`') return _codeSpanClose(src, i);
  if (c == '<') {
    // Autolink first — it constrains the closing `>` — then raw HTML. An
    // autolink holds no whitespace and no `<`, so the scan stops there
    // (mirrors Rust): running to the next `>` anywhere made `[<[<…` cubic.
    var close = -1;
    for (var j = i + 1; j < src.length; j++) {
      final d = src[j];
      if (d == '>') {
        close = j;
        break;
      }
      if (d == '<' || d == ' ' || d == '\t' || d == '\n' || d == '\r') break;
    }
    if (close > i && autolinkTarget(src.substring(i + 1, close)) != null) {
      return close + 1;
    }
    final html = inlineHtmlEnd(src, i);
    return html > i ? html : -1;
  }
  return -1;
}

/// Line-ending canonicalization (§6.7/6.9/6.12) with span awareness: a run of
/// 2+ trailing spaces before `\n` OUTSIDE code spans / autolinks / raw inline
/// HTML is a hard break — rewritten to the `\`+newline stored-text convention;
/// a soft break just drops the trailing whitespace. INSIDE those spans every
/// byte is content: ``` `code␠␠⏎span` ``` keeps its spaces (the code-span
/// handler maps `\n` to a space).
///
/// This lives at the INLINE layer because only it can see code spans — a span
/// may straddle the line ending, so the block layer, which works line by line,
/// cannot decide. It used to decide anyway (`endsHard` on the open block) and
/// injected a literal backslash into code content. Mirrors the Rust engine's
/// `parse_inline_memo` preamble. Idempotent, so the recursive re-parse of link
/// labels is safe.
String _canonicalizeBreaks(String src) {
  const sp = 0x20, tab = 0x09, nl = 0x0a, backslash = 0x5c;
  final out = <int>[];
  var i = 0;
  while (i < src.length) {
    final c = src.codeUnitAt(i);
    // Escaped char (incl. `` \` ``): never a span opener; copy as-is. A
    // backslash directly before `\n` falls through — it IS a hard break.
    if (c == backslash && i + 1 < src.length && src.codeUnitAt(i + 1) != nl) {
      out.add(c);
      out.add(src.codeUnitAt(i + 1));
      i += 2;
      continue;
    }
    if (c == 0x60 /* ` */ || c == 0x3c /* < */ ) {
      final end = _inlineSpanEnd(src, i);
      if (end > i) {
        for (var k = i; k < end; k++) {
          out.add(src.codeUnitAt(k));
        }
        i = end;
        continue;
      }
    }
    if (c == nl) {
      // Hard break = 2+ SPACES immediately before the newline; any other
      // trailing whitespace (tabs included) just drops with a soft break.
      var spaces = 0;
      while (spaces < out.length && out[out.length - 1 - spaces] == sp) {
        spaces++;
      }
      var ws = spaces;
      while (ws < out.length &&
          (out[out.length - 1 - ws] == sp || out[out.length - 1 - ws] == tab)) {
        ws++;
      }
      out.removeRange(out.length - ws, out.length);
      if (spaces >= 2) out.add(backslash);
      out.add(nl);
      i++;
      continue;
    }
    out.add(c);
    i++;
  }
  // End-of-text trailing whitespace is outside spans by construction.
  while (out.isNotEmpty && (out.last == sp || out.last == tab)) {
    out.removeLast();
  }
  return String.fromCharCodes(out);
}

/// The `]` matching the `[` at [open], honoring nesting and escapes; -1 if
/// none.
///
/// Code spans, autolinks and raw inline HTML bind tighter than link brackets
/// (CommonMark §6.5), so a `]` inside one of those does not close the link.
/// Mirrors Rust `matching_bracket`; without the skip, ``[`a]b`](/x)`` lost
/// its link entirely on the Dart side (confirmed drift).
int matchingBracket(String src, int open) {
  // Answers memoised per source string (mirrors Rust `BracketMemo`): an
  // unmatched `[` scans to the end, and the parser asks for every `[`, so
  // `[<[<[<…` cost a full scan per bracket — quadratic. One scan records, via
  // a stack, the answer for every later `[` it passes; the walk from any
  // position is deterministic, so those answers are exact.
  // The same scan records each matched `[`'s NESTING (bracket levels opening
  // inside it) for [bracketNesting] — mirrors Rust `BracketMemo::nesting`.
  if (!identical(src, _bracketMemoSrc)) {
    _bracketMemoSrc = src;
    _bracketMemo.clear();
    _bracketNest.clear();
  }
  final known = _bracketMemo[open];
  if (known != null) return known;
  final stack = <int>[];
  // Deepest nesting seen inside each open `[` on [stack], index-aligned.
  final inner = <int>[];
  var result = -1;
  var j = open;
  while (j < src.length) {
    final c = src[j];
    if (c == r'\' && j + 1 < src.length) {
      j += 2;
      continue;
    }
    final span = _inlineSpanEnd(src, j);
    if (span > j) {
      j = span;
      continue;
    }
    if (c == '[') {
      stack.add(j);
      inner.add(0);
    } else if (c == ']' && stack.isNotEmpty) {
      final o = stack.removeLast();
      final depth = inner.removeLast();
      _bracketMemo[o] = j;
      _bracketNest[o] = depth;
      if (inner.isNotEmpty && inner.last < depth + 1) inner.last = depth + 1;
      if (o == open) {
        result = j;
        break;
      }
    }
    j++;
  }
  if (result < 0) {
    for (final o in stack) {
      _bracketMemo[o] = -1;
    }
    _bracketMemo[open] = -1;
  }
  return result;
}

String? _bracketMemoSrc;
final Map<int, int> _bracketMemo = {};
final Map<int, int> _bracketNest = {};

/// Bracket levels nested inside the `[` at [open] (0 for `[a]`, 1 for
/// `[[a]]`); 0 when it has no match. Mirrors Rust `BracketMemo::nesting`.
int bracketNesting(String src, int open) {
  if (matchingBracket(src, open) < 0) return 0;
  return _bracketNest[open] ?? 0;
}

/// Per-source "next occurrence from position i" tables for the inline
/// scanner's closer searches. Issue #61: an opener without a closer made its
/// search run to the END of the string, and the scanner tries an opener at
/// (nearly) every position — unclosed `\(`, `<…>` autolinks, `$…$` math,
/// backtick runs and GFM autolink probes were each O(N) per position and
/// O(N²) per parse. Every answer here is a pure function of (src, position),
/// so one right-to-left pass records it for all positions and later lookups
/// are O(1). Behavior is unchanged by construction; the tables only replace
/// WHERE the linear scan happens (once, ahead of time) — same fix class as
/// the Rust engine's pre-scan memoization.
class _ScanMemo {
  _ScanMemo(this.src);

  final String src;

  static final RegExp _digit = RegExp(r'[0-9]');

  List<int>? _closeParen; // next `\)` starting at/after i
  List<int>? _gt; //         next '>'
  List<int>? _nl; //         next '\n'
  List<int>? _backtick; //   next '`'
  List<int>? _stop; //       next ' '/'\t'/'\n'/'<' (extended-autolink run end)
  List<int>? _at; //         next '@'
  List<int>? _dollarCloser; // next '$' passing the local Pandoc closer rules
  List<int>? _dquote; //     next '"'
  List<int>? _squote; //     next "'"
  List<int>? _emailLocalBreak; //  first index NOT in [A-Za-z0-9._+-]
  List<int>? _emailDomainBreak; // first index NOT in [A-Za-z0-9._-]
  List<int>? _codeSpanAns; // codeSpanClose() answer, per backtick-RUN START
  final Map<String, List<int>> _patterns =
      {}; // inline-HTML tail patterns (`-->`, `]]>`, `?>`)

  /// The first `@` whose email-candidate validation failed. The regex
  /// candidate's TAIL (everything after the `@`) is identical for every word
  /// boundary inside the same local-part run, and trimming only touches that
  /// tail — so the lookahead verdict (`@` present, domain has a dot, ends
  /// alphanumeric) is the same for all of them, and re-running the regex at
  /// each boundary of `_a_a_a_…@…` was pure waste (issue #61).
  int emailFailAt = -1;

  List<int> _nextTable(bool Function(int i) matches) {
    final t = List<int>.filled(src.length + 1, -1);
    var next = -1;
    for (var i = src.length - 1; i >= 0; i--) {
      if (matches(i)) next = i;
      t[i] = next;
    }
    return t;
  }

  int _lookup(List<int> t, int from) => from >= src.length ? -1 : t[from];

  int closeParenFrom(int from) => _lookup(
    _closeParen ??= _nextTable(
      (i) =>
          i + 1 < src.length &&
          src.codeUnitAt(i) == 0x5C /* \ */ &&
          src.codeUnitAt(i + 1) == 0x29 /* ) */,
    ),
    from,
  );

  int gtFrom(int from) =>
      _lookup(_gt ??= _nextTable((i) => src[i] == '>'), from);

  int nlFrom(int from) =>
      _lookup(_nl ??= _nextTable((i) => src[i] == '\n'), from);

  int backtickFrom(int from) =>
      _lookup(_backtick ??= _nextTable((i) => src[i] == '`'), from);

  int atFrom(int from) =>
      _lookup(_at ??= _nextTable((i) => src[i] == '@'), from);

  int patternFrom(int from, String pat) => _lookup(
    _patterns.putIfAbsent(pat, () => _nextTable((i) => src.startsWith(pat, i))),
    from,
  );

  int quoteFrom(int from, String quote) => quote == '"'
      ? _lookup(_dquote ??= _nextTable((i) => src[i] == '"'), from)
      : _lookup(_squote ??= _nextTable((i) => src[i] == "'"), from);

  /// First index ≥ i of an extended-autolink stop char (space/tab/newline/
  /// `<`), or src.length — same set the old hand-rolled scan used.
  int autolinkStopAt(int i) {
    final v = _lookup(
      _stop ??= _nextTable(
        (j) =>
            src[j] == ' ' || src[j] == '\t' || src[j] == '\n' || src[j] == '<',
      ),
      i,
    );
    return v < 0 ? src.length : v;
  }

  /// Next `$` at/after [from] that passes the LOCAL Pandoc closer checks (not
  /// preceded by whitespace/backslash, not followed by a digit). Whether it
  /// actually closes math from a given opener additionally depends on
  /// newlines and code spans between them — [_findMathCloser] combines these.
  int dollarCloserFrom(int from) => _lookup(
    _dollarCloser ??= _nextTable(
      (i) =>
          src[i] == kDollar &&
          i > 0 &&
          !isUnicodeWhitespace(src[i - 1]) &&
          src[i - 1] != r'\' &&
          (i + 1 >= src.length || !_digit.hasMatch(src[i + 1])),
    ),
    from,
  );

  static bool _isAsciiAlnum(String c) {
    final u = c.codeUnitAt(0);
    return (u | 0x20) >= 0x61 && (u | 0x20) <= 0x7A || (u >= 0x30 && u <= 0x39);
  }

  List<int> _breakTable(bool Function(String c) inClass) {
    final t = List<int>.filled(src.length + 1, src.length);
    for (var i = src.length - 1; i >= 0; i--) {
      t[i] = inClass(src[i]) ? t[i + 1] : i;
    }
    return t;
  }

  /// First index ≥ i NOT in the email LOCAL-part class `[A-Za-z0-9._+-]`
  /// (src.length when the run reaches the end).
  int emailLocalBreakAt(int i) => i >= src.length
      ? src.length
      : (_emailLocalBreak ??= _breakTable(
          (c) =>
              _isAsciiAlnum(c) || c == '.' || c == '_' || c == '+' || c == '-',
        ))[i];

  /// First index ≥ i NOT in the email DOMAIN class `[A-Za-z0-9._-]`.
  int emailDomainBreakAt(int i) => i >= src.length
      ? src.length
      : (_emailDomainBreak ??= _breakTable(
          (c) => _isAsciiAlnum(c) || c == '.' || c == '_' || c == '-',
        ))[i];

  /// One past the closing run of equal length for the backtick run STARTING
  /// at [start], or -1 — the full answer of `_codeSpanClose`. One
  /// right-to-left pass keeps, per run length, the next run start of that
  /// length: that run (plus its length) is exactly what the old scan returned
  /// — an unclosed run no longer rescans to the end for every later opener
  /// (part of issue #61).
  int codeSpanClose(int start) =>
      (_codeSpanAns ??= _buildCodeSpanClose())[start];

  List<int> _buildCodeSpanClose() {
    final n = src.length;
    final ans = List<int>.filled(n, -1);
    final lastRunStartByLen = <int, int>{};
    var i = n - 1;
    while (i >= 0) {
      if (src[i] != '`') {
        i--;
        continue;
      }
      var s = i;
      while (s > 0 && src[s - 1] == '`') {
        s--;
      }
      final len = i - s + 1;
      final later = lastRunStartByLen[len];
      ans[s] = later == null ? -1 : later + len;
      lastRunStartByLen[len] = s;
      i = s - 1;
    }
    return ans;
  }
}

/// Memos keyed by source string CONTENT, bounded: the scanner alternates
/// between a paragraph and the recursive parses of its link labels, so a
/// single slot (like the bracket memo's) would evict-and-rebuild on every
/// alternation. Rebuilds are one linear pass each, only for tables actually
/// used.
const int _kScanMemoSlots = 8;
final Map<String, _ScanMemo> _scanMemos = {};

_ScanMemo _scanMemoFor(String src) {
  final hit = _scanMemos[src];
  if (hit != null) return hit;
  final memo = _ScanMemo(src);
  if (_scanMemos.length >= _kScanMemoSlots) {
    _scanMemos.remove(_scanMemos.keys.first);
  }
  _scanMemos[src] = memo;
  return memo;
}

/// Link / image labels nested deeper than this are not parsed as links or
/// images: their brackets stay literal text (the content inside still parses).
/// Mirrors Rust `MAX_INLINE_NESTING` (`crates/markdown/src/inline.rs`) —
/// change both together.
///
/// Every recognized label is parsed by a recursive call, so nesting was
/// recursion depth: `![![![…a](u)](u)](u)` thousands deep overflowed the stack.
/// The bound is structural — a label is allowed only while fewer than this
/// many bracket levels open inside it ([bracketNesting]) — so it depends on
/// the label text alone and the label cache stays a pure memo.
const int kMaxInlineNesting = 32;

const kDollar = r'$';

/// Unicode `White_Space` — exactly the set Rust's `char::is_whitespace`
/// answers, which the authoritative engine uses at the autolink boundary and
/// the math delimiters. This mirror used to test only ASCII space/tab/newline,
/// so `中文\u3000www.example.com` autolinked in Rust but not here, and
/// `$x\u3000$` was math here but not there.
bool isUnicodeWhitespace(String c) {
  if (c.isEmpty) return false;
  final u = c.codeUnitAt(0);
  return (u >= 0x09 && u <= 0x0D) ||
      u == 0x20 ||
      u == 0x85 ||
      u == 0xA0 ||
      u == 0x1680 ||
      (u >= 0x2000 && u <= 0x200A) ||
      u == 0x2028 ||
      u == 0x2029 ||
      u == 0x202F ||
      u == 0x205F ||
      u == 0x3000;
}

/// One past the closing backtick run of the code span opening at [start] (the
/// index of its first backtick), or -1 when the run never closes — CommonMark
/// leaves an unclosed run as literal text.
///
/// Mirrors `code_span_close` in `crates/markdown/src/lib.rs` (Rust is
/// authoritative — CLAUDE.md #2). Deliberately separate from the inline-code
/// scanner, which also strips padding and folds newlines; this only needs the
/// extent.
///
/// Answers come from the [_ScanMemo] run table: an unclosed run used to
/// rescan to the end of the string for EVERY later opener (issue #61).
int _codeSpanClose(String src, int start) {
  if (start > 0 && src[start - 1] == '`') {
    // The memo is keyed on run STARTS; a mid-run start is only reachable
    // through an escape-skip landing inside a run — take the plain scan.
    return _codeSpanCloseForward(src, start);
  }
  return _scanMemoFor(src).codeSpanClose(start);
}

/// The plain forward scan of [_codeSpanClose], kept for mid-run starts (the
/// memo table only holds run starts).
int _codeSpanCloseForward(String src, int start) {
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
///
/// Fast path via [_ScanMemo]: the walk can only succeed at a `$` passing the
/// LOCAL closer checks (prev not whitespace/backslash, next not a digit), so
/// the memoised next-valid-closer, next-newline and next-backtick positions
/// decide the outcome in O(1) whenever no backtick region intervenes:
///
/// - no valid closer at all → the walk necessarily fails;
/// - a newline before that closer → the walk hits it and fails (a code span
///   straddling the newline fails the same way);
/// - no backtick in between → the walk reaches that closer untouched.
///
/// Only the backtick case needs the span-aware walk. Failed searches used to
/// rescan the whole tail at every `$` — `$a $b $c …` was quadratic (#61).
int _findMathCloser(String src, int contentStart) {
  if (contentStart >= src.length) return -1;
  final first = src[contentStart];
  if (isUnicodeWhitespace(first)) return -1;
  final memo = _scanMemoFor(src);
  final close = memo.dollarCloserFrom(contentStart + 1);
  if (close < 0) return -1;
  final nl = memo.nlFrom(contentStart);
  if (nl >= 0 && nl < close) return -1;
  final bt = memo.backtickFrom(contentStart);
  if (bt < 0 || bt >= close) return close;
  return _findMathCloserWalk(src, contentStart);
}

/// The span-aware walk of [_findMathCloser], used when a backtick region lies
/// between opener and the table-derived closer (a `$` inside a code span is
/// literal and cannot close).
int _findMathCloserWalk(String src, int contentStart) {
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
      if (isUnicodeWhitespace(prev) || prev == r'\') {
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
      // Memoised next-`\)` lookup (issue #61): an unclosed `\(` used to make
      // indexOf scan to the end of the string at every opener.
      final close = _scanMemoFor(src).closeParenFrom(i + 2);
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
      // Memoised next-`\)` lookup (issue #61): an unclosed `\(` used to make
      // indexOf scan to the end of the string at every opener.
      final close = _scanMemoFor(src).closeParenFrom(i + 2);
      if (close > i && src.substring(i + 2, close).trim().isNotEmpty) {
        spans.add((start: i, end: close + 2));
        i = close + 2;
        continue;
      }
    }
    // A code span binds tighter than math (§6.1) — step over it whole so a `$`
    // inside cannot open a run. Mirrors Rust `math_run_spans`; without it
    // "`$HOME` and $x$" produced one bogus span from the first `$`.
    if (src[i] == '`') {
      final after = _codeSpanClose(src, i);
      if (after > i) {
        i = after;
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
  // Only the first 8 chars decide the prefix; copying and lowercasing the
  // whole remaining text at every word boundary made parsing quadratic (and a
  // re-parsed multi-line paragraph cubic). Mirrors Rust `extended_autolink`.
  final lower = src.substring(i, (i + 8).clamp(i, src.length)).toLowerCase();
  for (final (prefix, implied) in [
    ('https://', ''),
    ('http://', ''),
    ('ftp://', ''),
    ('www.', 'http://'),
  ]) {
    if (lower.startsWith(prefix)) {
      // The run-end lookup is memoised (issue #61): the old per-boundary scan
      // for the next space/tab/newline/`<` ran to the end on run-dense input.
      final stop = _scanMemoFor(src).autolinkStopAt(i);
      final rest = src.substring(i, stop);
      final end = _trimAutolinkEnd(rest);
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
  // Bare email. Two memoised gates issue #61's `_a_a_a_…` vector, where the
  // regex rescanned the whole tail at every word boundary:
  //
  // 1. A match needs an `@` IMMEDIATELY after the local-part run — no `@`, or
  //    anything else breaking the run first, fails in O(1) without the regex.
  //    (The classes exclude `@`, so next-at can't lie inside the run.)
  // 2. Every boundary inside one local-part run asks the same question, and
  //    the validation verdict depends only on the candidate's tail after the
  //    `@` — identical for all of them — so a failure is remembered per `@`.
  if (RegExp(r'[A-Za-z0-9]').hasMatch(src[i])) {
    final memo = _scanMemoFor(src);
    final at = memo.atFrom(i);
    if (at == memo.emailLocalBreakAt(i) &&
        memo.emailDomainBreakAt(at + 1) > at + 1 &&
        memo.emailFailAt != at) {
      final m = RegExp(
        r'[A-Za-z0-9._+-]+@[A-Za-z0-9._-]+',
      ).matchAsPrefix(src, i);
      if (m != null) {
        var text = m.group(0)!;
        while (text.endsWith('.')) {
          text = text.substring(0, text.length - 1);
        }
        final atInText = text.indexOf('@');
        final domain = atInText >= 0 ? text.substring(atInText + 1) : '';
        if (atInText > 0 &&
            domain.contains('.') &&
            RegExp(r'[A-Za-z0-9]$').hasMatch(text)) {
          return (text.length, 'mailto:$text');
        }
      }
      memo.emailFailAt = at;
    }
  }
  return null;
}

int _trimAutolinkEnd(String s) {
  final n = s.length;
  if (n == 0) return 0;
  // Index-based walk over prefix-count tables: the old version rebuilt `kept`
  // and rescanned it for parens/`&` on EVERY trim step — O(len) per step,
  // O(len²) on `www.a))))…` — while only ever asking prefix questions
  // (paren balance over s[0..e), last `&` before e). Same answers, one pass.
  final opens = List<int>.filled(n + 1, 0);
  final closes = List<int>.filled(n + 1, 0);
  final prevAmp = List<int>.filled(n + 1, -1);
  final alnum = RegExp(r'[A-Za-z0-9]');
  for (var k = 0; k < n; k++) {
    opens[k + 1] = opens[k] + (s[k] == '(' ? 1 : 0);
    closes[k + 1] = closes[k] + (s[k] == ')' ? 1 : 0);
    prevAmp[k + 1] = s[k] == '&' ? k : prevAmp[k];
  }
  var end = n;
  while (end > 0) {
    final last = s[end - 1];
    if ('?!.,:*_~\'"'.contains(last)) {
      end -= 1;
      continue;
    }
    if (last == ')') {
      if (closes[end] > opens[end]) {
        end -= 1;
        continue;
      }
      return end;
    }
    if (last == ';') {
      final amp = prevAmp[end];
      if (amp >= 0) {
        // body = s[amp + 1 .. end - 1)
        var bodyOk = end - amp > 2;
        for (var k = amp + 1; bodyOk && k < end - 1; k++) {
          if (!alnum.hasMatch(s[k])) bodyOk = false;
        }
        if (bodyOk) {
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
  // Pattern/quote searches go through the memoised next-occurrence tables
  // (issue #61): an unterminated comment or attribute value otherwise scanned
  // to the end of the string on every `<` the parser tries.
  final memo = _scanMemoFor(src);
  int findPat(int from, String pat) =>
      pat == '>' ? memo.gtFrom(from) : memo.patternFrom(from, pat);
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
        final w = memo.quoteFrom(v + 1, quote);
        if (w < 0) return -1;
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

/// Does this link label already contain a link? CommonMark §6.3 forbids links
/// inside link text, so when true the surrounding brackets stay literal. Images
/// are fine (`[![alt](img)](url)`), so only `link` marks count.
///
/// Goes through the label cache ([_parseLabelCached]), shared with [addLink].
bool _labelHasLinkCached(
  String label,
  Map<String, ({String dest, String? title})> defs,
  _LabelCache cache,
) {
  // `allowAutolink: false` (inside [_parseLabelCached]) is the whole point.
  //
  // CommonMark forbids a link inside a link, and this guard enforces it — but
  // it used to ask "does the label parse to anything with a link mark", and a
  // GFM *extended autolink* produces exactly that mark. So a label that was
  // itself a bare URL counted as a nested link, and `[https://x](https://x)`
  // — precisely what the clipboard hands back when you copy a link out of Mica
  // and paste it in again — was refused and fell through to literal text. The
  // nesting rule is about explicit link CONSTRUCTS, not about text that would
  // have autolinked had it been left alone. Mirrors `label_has_link_cached` in
  // crates/markdown/src/inline.rs.
  return _parseLabelCached(
    label,
    defs,
    cache,
  ).marks.any((m) => m.type == 'link');
}

/// Parsed link/image LABELS, keyed by label text and SHARED across the
/// recursive parses of one top-level parse — by the nested-link check AND by
/// `addLink`, which both ask the same pure question. Mirrors Rust `LinkCache`.
///
/// Without sharing, a label that DOES contain a link is parsed twice per
/// nesting level (the check, then the scan falling through the rejected outer
/// brackets): T(n) = 2·T(n-1) on `[[[a](/u)](/u)](/u)`. That was fixed for the
/// check alone; `addLink` still parsed through a FRESH cache, so links and
/// images ALTERNATING (`[![[![…a](u)](u)](u)](u)`) re-ran every check below
/// an image: P(k) = 2·P(k-2) (P1-7).
typedef _LabelCache = Map<String, ({String text, List<Mark> marks})>;

/// Parse a link's (or image's) LABEL through the shared [_LabelCache].
///
/// Autolinks off: the label is about to become the link's TEXT, and text inside
/// a link never autolinks. With it on, a URL-shaped label got a second link
/// mark of its own — `[https://a](https://b)` produced TWO overlapping marks
/// over the same range, one pointing at `a` and one at `b`, and which one wins
/// becomes a question about mark order that has no right answer.
/// Pure function + pure cache: output cannot change. Mirrors Rust
/// `parse_label_cached`.
({String text, List<Mark> marks}) _parseLabelCached(
  String label,
  Map<String, ({String dest, String? title})> defs,
  _LabelCache cache,
) {
  final hit = cache[label];
  if (hit != null) return hit;
  final parsed = _parseInlineMemo(label, defs, cache, allowAutolink: false);
  cache[label] = parsed;
  return parsed;
}

/// Parse inline Markdown (`**b**`, `*i*`/`_i_`, `` `c` ``, `~~s~~`,
/// `[t](url)`, `\*escapes\*`, `<autolinks>`) into clean text plus marks.
({String text, List<Mark> marks}) parseInline(
  String src, {
  Map<String, ({String dest, String? title})> defs = const {},
}) => _parseInlineMemo(src, defs, {});

({String text, List<Mark> marks}) _parseInlineMemo(
  String rawSrc,
  Map<String, ({String dest, String? title})> defs,
  _LabelCache cache, {
  // Whether GFM extended autolinks (bare `www.`/`http(s)://`/email) are
  // recognized. Off ONLY while parsing a link LABEL — see
  // [_labelHasLinkCached] and [_parseLabelCached].
  bool allowAutolink = true,
}) {
  final src = _canonicalizeBreaks(rawSrc);
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
    final parsed = _parseLabelCached(label, defs, cache);
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
      // Memoised next-`\)` lookup (issue #61): an unclosed `\(` used to make
      // indexOf scan to the end of the string at every opener.
      final close = _scanMemoFor(src).closeParenFrom(i + 2);
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
      // Memoised next-'>' lookup: without it every unclosed `<` scanned to
      // the end of the string (issue #61).
      final close = _scanMemoFor(src).gtFrom(i + 1);
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
    // Past `kMaxInlineNesting` the brackets stay literal (see there).
    if (src[i] == '!' && i + 1 < src.length && src[i + 1] == '[') {
      final close = matchingBracket(src, i + 1);
      if (close > i + 2 && bracketNesting(src, i + 1) < kMaxInlineNesting) {
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
      if (close > i && bracketNesting(src, i) < kMaxInlineNesting) {
        final label = src.substring(i + 1, close);
        // CommonMark §6.3: a link's text may not itself contain a link. When
        // the label already holds one, the OUTER brackets stay literal and the
        // inner link wins — `[foo [bar](/uri)](/baz)` is `[foo <a>bar</a>](/baz)`,
        // not a nested anchor. Images are fine (`[![alt](img)](url)` is a valid
        // linked image), so only `link` marks disqualify. Mirrors Rust
        // `label_contains_link`; without it the Dart mirror produced a different
        // mark range than the engine. (P1-2 confirmed drift.)
        // The nested-link check is LAZY (it only gates the two link forms below)
        // and memoized through a cache SHARED across the whole parse — see
        // `_labelHasLinkCached`. Together they turn parsing from exponential to
        // flat on nested input, which matters because this runs on the editor's
        // PASTE path — the old behavior was a UI freeze on crafted content.
        // Measured, before -> after: `[[[[a]]]]` depth 24 ~12s -> 0ms;
        // `[[[a](/u)](/u)](/u)` depth 32 -> 4ms.
        //
        // Output is unchanged (the shared conformance fixtures pin that against
        // the Rust engine); pinned for speed by nested_bracket_perf_test.dart.
        if (close + 1 < src.length && src[close + 1] == '(') {
          final suffix = parseLinkSuffix(src, close + 2);
          if (suffix != null && !_labelHasLinkCached(label, defs, cache)) {
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
          if (def != null && !_labelHasLinkCached(label, defs, cache)) {
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
        isUnicodeWhitespace(src[i - 1]) ||
        '*_~('.contains(src[i - 1])) {
      final auto = allowAutolink ? extendedAutolink(src, i) : null;
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
        // A SINGLE tilde may not open or close INTRAWORD. Mirror of the Rust
        // rule (crates/markdown, `is_word_char`) — see the long note there for
        // why this deliberately diverges from GitHub: the tildes are consumed
        // into a mark here, and a range like "1~2 个 Token" written twice in
        // one paragraph would silently eat everything between.
        bool intraword(int at, int run) =>
            at > 0 &&
            _isWordChar(src[at - 1]) &&
            at + run < src.length &&
            _isWordChar(src[at + run]);
        var j = i + n;
        var close = -1;
        while (j < src.length && !(n == 1 && intraword(i, n))) {
          if (src[j] == '~' && (j == 0 || src[j - 1] != r'\')) {
            var m = 0;
            while (j + m < src.length && src[j + m] == '~') {
              m++;
            }
            if (m == n && j > i + n && !(n == 1 && intraword(j, m))) {
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
      // Same search as _codeSpanClose, through its memoised run table — an
      // unclosed run no longer rescans the tail per opener (issue #61). The
      // helper answers one-past the closing run; this branch needs its start.
      final after = _codeSpanClose(src, i);
      final close = after >= 0 ? after - n : -1;
      if (close >= 0) {
        var inner = src.substring(i + n, close).replaceAll('\n', ' ');
        // The "content isn't blank" guard is «not all SPACES» (§6.1), not
        // «not all whitespace»: `` ` \t ` `` strips to a tab. `trim()` also
        // eats tabs, so it answered "blank" and skipped the strip — a
        // confirmed drift from Rust's `!inner.bytes().all(|b| b == b' ')`.
        if (inner.length >= 2 &&
            inner.startsWith(' ') &&
            inner.endsWith(' ') &&
            inner.codeUnits.any((u) => u != 0x20)) {
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

/// A "word" character for the single-tilde intraword rule: neither whitespace
/// nor punctuation. Mirror of the Rust `is_word_char` (crates/markdown).
///
/// CJK counts as a WORD character here, unlike in [_flanking], where the CJK
/// amendment treats it as a boundary — that amendment is about PUNCTUATION
/// adjacency (`**加粗。**后文`) and has no analogue for `~`. Counting CJK as
/// word is what makes `一~二` behave like `1~2`.
bool _isWordChar(String c) => c.trim().isNotEmpty && !_isMdPunct(c);

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
/// (docs/reviews/code-review-2026-07-20.md P1-1.)
String escapeBlockLeader(String line) {
  // Divider: strip spaces first (`-- -` is a thematic break), then >=3 dashes.
  final compact = line.replaceAll(' ', '');
  final dividerLike =
      compact.length >= 3 && compact.split('').every((c) => c == '-');
  // Setext underline: a whole line of `=` or of `-` underlines the paragraph
  // above it as a heading on re-parse.
  final setextLike =
      line.isNotEmpty &&
      (line.split('').every((c) => c == '=') ||
          line.split('').every((c) => c == '-'));
  // A marker is followed by a space, a tab, or nothing (an empty item /
  // empty heading is still that block).
  bool markerThenGap(String rest) =>
      rest.isEmpty || rest.startsWith(' ') || rest.startsWith('\t');
  // ATX: `#`+ then a gap (any count, matching Rust — not just 1–6).
  final atxLike =
      line.startsWith('#') &&
      markerThenGap(line.replaceFirst(RegExp(r'^#+'), ''));
  if (line.startsWith('>') ||
      ((line.startsWith('-') || line.startsWith('+')) &&
          markerThenGap(line.substring(1))) ||
      dividerLike ||
      setextLike ||
      atxLike) {
    return '\\$line';
  }
  // `\[` opens display math on re-import and [escapeInline] writes every
  // literal `[` as `\[`: a paragraph `[TODO]` came back as a math block. An
  // entity reference is plain text to the inline parser. Only where the math
  // block would actually open (`\[` alone, or a line ending in `\]`).
  if (line.startsWith(r'\[')) {
    final rest = line.substring(2);
    if (rest.trim().isEmpty || rest.trimRight().endsWith(r'\]')) {
      return '&#91;$rest';
    }
  }
  final numbered = RegExp(r'^(\d{1,9})([.)])').firstMatch(line);
  if (numbered != null && markerThenGap(line.substring(numbered.end))) {
    return '${numbered.group(1)}\\${numbered.group(2)}'
        '${line.substring(numbered.end)}';
  }
  return line;
}

/// [escapeBlockLeader] on every line — after a hard break a later line sits
/// at a line start just as much as the first. Mirrors Rust
/// `escape_block_leaders`.
String escapeBlockLeaders(String text) =>
    text.split('\n').map(escapeBlockLeader).join('\n');

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
  final sorted = _sortedForRender(marks);
  return _renderSpan(text, 0, text.length, sorted, sorted.length, {});
}

/// Marks whose rendering is TERMINAL: the branch writes the span literally and
/// cannot nest anything inside it, so it must never win an exact-range tie — it
/// would swallow the mark it coincides with. ``[`a`](/x)`` used to export as
/// `` `a` ``, dropping the link outright (and with it the URL).
/// Mirrors Rust `renders_terminal`.
bool _rendersTerminal(String kind) =>
    kind == 'code' || kind == 'math' || kind == 'html' || kind == 'footnote';

/// A [Mark] plus its index in the caller's list. The index reproduces the old
/// per-span min-over-all-marks scan's tie-breaking exactly — it matters
/// because the parser APPENDS outer marks later for identical ranges.
class _RenderMark {
  _RenderMark(this.mark, this.index);

  final Mark mark;
  final int index;

  bool? _term;
  bool get term => _term ??= _rendersTerminal(mark.type);
}

/// Marks prepared for rendering: sorted by start (ties keep caller order) so
/// the span picker streams candidates instead of rescans of the whole list at
/// every span — issue #61's O(M²) per-span minimisation.
List<_RenderMark> _sortedForRender(List<Mark> marks) {
  final wrapped = [
    for (var i = 0; i < marks.length; i++) _RenderMark(marks[i], i),
  ];
  wrapped.sort((a, b) {
    if (a.mark.start != b.mark.start) return a.mark.start - b.mark.start;
    return a.index - b.index;
  });
  return wrapped;
}

/// First index in `marks[0..count)` whose start is >= [start] (binary search;
/// the list is start-sorted). Marks starting at or after a span's end cannot
/// overlap it, so a recursion level only ever needs this prefix — the old
/// code filtered a fresh `inner` list per span instead.
int _startBoundBelow(List<_RenderMark> marks, int count, int start) {
  var lo = 0, hi = count;
  while (lo < hi) {
    final mid = (lo + hi) >> 1;
    if (marks[mid].mark.start < start) {
      lo = mid + 1;
    } else {
      hi = mid;
    }
  }
  return lo;
}

/// Heap of the marks currently covering the render position of one render
/// level, best pick on top. "Best" reproduces the old scan's exact ordering:
/// widest clipped end wins; exact (start, end) ties prefer the non-terminal
/// kind (Markdown only), then the LATEST mark in the caller's list.
class _ActiveMarks {
  _ActiveMarks({required this.hi, required this.termRank});

  final int hi;
  final bool termRank;
  final List<_RenderMark> _items = [];

  bool get isEmpty => _items.isEmpty;

  int _clipEnd(_RenderMark m) => m.mark.end > hi ? hi : m.mark.end;

  bool _better(_RenderMark a, _RenderMark b) {
    final ea = _clipEnd(a), eb = _clipEnd(b);
    if (ea != eb) return ea > eb;
    if (termRank && a.term != b.term) return !a.term;
    return a.index > b.index;
  }

  void add(_RenderMark m) {
    _items.add(m);
    var i = _items.length - 1;
    while (i > 0) {
      final parent = (i - 1) >> 1;
      if (!_better(_items[i], _items[parent])) break;
      final tmp = _items[parent];
      _items[parent] = _items[i];
      _items[i] = tmp;
      i = parent;
    }
  }

  void removeTop() {
    final last = _items.removeLast();
    if (_items.isEmpty) return;
    _items[0] = last;
    var i = 0;
    while (true) {
      var best = i;
      final l = 2 * i + 1, r = l + 1;
      if (l < _items.length && _better(_items[l], _items[best])) best = l;
      if (r < _items.length && _better(_items[r], _items[best])) best = r;
      if (best == i) return;
      final tmp = _items[best];
      _items[best] = _items[i];
      _items[i] = tmp;
      i = best;
    }
  }

  /// The best live mark at [pos] (widest clipped end > pos), or null.
  _RenderMark? topAt(int pos) {
    while (_items.isNotEmpty && _clipEnd(_items.first) <= pos) {
      removeTop();
    }
    return _items.isEmpty ? null : _items.first;
  }
}

/// One level of a [_renderSpan] recursion: streams the mark opening each
/// successive span in exactly the order the old min-over-all-marks scan
/// picked them. Candidates come from the start-sorted list: marks already
/// started live in an [_ActiveMarks] heap; when none are live, the next span
/// opens at the next candidate's own start.
class _RenderLevel {
  _RenderLevel(this.marks, this.count, this.hi, this.skip, this.termRank);

  final List<_RenderMark> marks;
  final int count;
  final int hi;

  /// Marks picked by an ANCESTOR recursion level and therefore ineligible
  /// here — the shared identity set the render recursion maintains (the old
  //  code passed a fresh list with its pick removed at every level instead).
  final Set<_RenderMark> skip;
  final bool termRank;

  late final _ActiveMarks _active = _ActiveMarks(hi: hi, termRank: termRank);
  var _k = 0; // next not-yet-activated candidate

  /// The mark opening the span at [pos], or null when no mark covers
  /// anything in [pos, hi) — its clipped start/end are ps/pe to the caller.
  _RenderMark? nextPick(int pos) {
    while (_k < count && marks[_k].mark.start <= pos) {
      final cand = marks[_k];
      if (!skip.contains(cand) && cand.mark.end > pos) _active.add(cand);
      _k++;
    }
    final top = _active.topAt(pos);
    if (top != null) return top;
    // Nothing live covers pos: the next span opens at the next mark's own
    // start. Marks SHARING that start all tied in the old scan, so the whole
    // group is ranked by the same rule; groups with no live member are dead.
    while (_k < count) {
      final s = marks[_k].mark.start;
      if (s >= hi) {
        _k = count;
        return null;
      }
      var gEnd = _k + 1;
      while (gEnd < count && marks[gEnd].mark.start == s) {
        gEnd++;
      }
      _RenderMark? best;
      for (var t = _k; t < gEnd; t++) {
        final cand = marks[t];
        if (skip.contains(cand)) continue;
        final e = cand.mark.end > hi ? hi : cand.mark.end;
        if (e <= s) continue;
        if (best == null || _active._better(cand, best)) best = cand;
      }
      if (best != null) return best;
      _k = gEnd;
    }
    return null;
  }
}

String _renderSpan(
  String text,
  int lo,
  int hi,
  List<_RenderMark> marks,
  int count,
  Set<_RenderMark> skip,
) {
  final out = StringBuffer();
  final level = _RenderLevel(marks, count, hi, skip, true);
  var pos = lo;
  while (pos < hi) {
    final pickW = level.nextPick(pos);
    if (pickW == null) {
      out.write(escapeInline(text.substring(pos, hi)));
      break;
    }
    final pick = pickW.mark;
    final ps = pick.start < pos ? pos : pick.start;
    final pe = pick.end > hi ? hi : pick.end;
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
    // Inner candidates are the shared start-sorted list up to the first mark
    // starting at/after pe (none of those overlap the pick). Marks that can't
    // clip into [ps, pe) are retired by the picker itself.
    final innerCount = _startBoundBelow(marks, count, pe);
    // Whether the pick has any inner mark to lose — gates the link
    // shorthands below, which DISCARD inner marks (``[`x`](x)`` must stay
    // bracketed).
    final hasInner = marks
        .take(innerCount)
        .any(
          (t) =>
              !identical(t, pickW) &&
              !skip.contains(t) &&
              t.mark.end > ps &&
              t.mark.start < pe,
        );
    skip.add(pickW);
    final body = _renderSpan(text, ps, pe, marks, innerCount, skip);
    skip.remove(pickW);
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
            !hasInner &&
            plain.startsWith('www.') &&
            href == 'http://$plain') {
          // A bare `www.` link writes back bare (GFM re-links it on read).
          out.write(plain);
        } else if (pick.title == null &&
            !hasInner &&
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
String inlineToHtml(String text, List<Mark> marks) {
  final sorted = _sortedForRender(marks);
  return _renderSpanHtml(text, 0, text.length, sorted, sorted.length, {});
}

String _renderSpanHtml(
  String text,
  int lo,
  int hi,
  List<_RenderMark> marks,
  int count,
  Set<_RenderMark> skip,
) {
  final out = StringBuffer();
  // Same streamed selection as _renderSpan; html's tie-break has no terminal
  // clause — on identical ranges the LATER mark in the caller's list wins
  // (the parser appends the outer mark last).
  final level = _RenderLevel(marks, count, hi, skip, false);
  var pos = lo;
  while (pos < hi) {
    final pickW = level.nextPick(pos);
    if (pickW == null) {
      out.write(escapeHtml(text.substring(pos, hi)));
      break;
    }
    final pick = pickW.mark;
    final ps = pick.start < pos ? pos : pick.start;
    final pe = pick.end > hi ? hi : pick.end;
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
    skip.add(pickW);
    final body = _renderSpanHtml(
      text,
      ps,
      pe,
      marks,
      _startBoundBelow(marks, count, pe),
      skip,
    );
    skip.remove(pickW);
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

/// Whether a click landing at caret [offset] should ACTIVATE link [m].
///
/// Not the same question as "is this offset inside the link". `offset ==
/// m.start` is the caret slot BEFORE the link's first character — clicking
/// there means "put the caret at the start of this line", which is the only way
/// to type above a line that begins with a link. Counting it as a hit made that
/// impossible: every click at the start opened the URL instead (reported
/// 2026-08-12, 「行首的超链接很难插入光标」). The renderer snaps to the nearest
/// boundary, so this gives the left half of the first glyph — plus the whole
/// left margin, which snaps there too — back to the caret.
///
/// A ONE-character link is the exception: `start` is then its only interior
/// slot, and excluding it would make such a link unclickable altogether.
bool linkClickHit(Mark m, int offset) {
  if (m.type != 'link' || m.href == null) return false;
  if (offset >= m.end) return false;
  final firstInterior = (m.end - m.start) > 1 ? m.start + 1 : m.start;
  return offset >= firstInterior;
}
