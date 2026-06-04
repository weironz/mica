import 'package:flutter/widgets.dart';

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

  static const types = {'bold', 'italic', 'code', 'strike', 'link'};
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
        marks.add(Mark(start, end, type,
            href: m['href'] as String?, title: m['title'] as String?));
      }
    }
  }
  return marks;
}

List<Map<String, dynamic>> marksToJson(List<Mark> marks) =>
    [for (final m in marks) m.toJson()];

const Color _codeColor = Color(0xFFB91C1C);
const Color _codeBg = Color(0x14B91C1C);
const Color _linkColor = Color(0xFF2563EB);

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
          style = style.copyWith(fontWeight: FontWeight.w700);
        case 'italic':
          style = style.copyWith(fontStyle: FontStyle.italic);
        case 'code':
          style = style.copyWith(
            fontFamily: 'monospace',
            color: _codeColor,
            backgroundColor: _codeBg,
          );
        case 'strike':
          decorations.add(TextDecoration.lineThrough);
        case 'link':
          style = style.copyWith(color: _linkColor);
          decorations.add(TextDecoration.underline);
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
  final sorted = [...marks]..sort((a, b) {
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
      out.add(Mark(prev.start, m.end > prev.end ? m.end : prev.end, m.type,
          href: m.href));
    } else {
      out.add(m);
    }
  }
  return out;
}

const String _asciiPunct = r'''!"#$%&'()*+,-./:;<=>?@[\]^_`{|}~''';

/// Index of the next [needle] at/after [from] that is not backslash-escaped.
int _indexOfUnescaped(String src, String needle, int from) {
  var j = from;
  while (true) {
    j = src.indexOf(needle, j);
    if (j < 0) return -1;
    if (j == 0 || src[j - 1] != r'\') return j;
    j += 1;
  }
}

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
  final rest = t.substring(close + 2).trim();
  if (rest.isEmpty) return null;
  // Reuse the suffix parser by appending a virtual `)` and requiring full
  // consumption.
  final suffix = parseLinkSuffix('$rest)', 0);
  if (suffix == null || suffix.next != rest.length + 1) return null;
  return (label: label, dest: suffix.dest, title: suffix.title);
}

/// Case-fold and collapse internal whitespace (spec label matching).
String normalizeLabel(String label) =>
    label.trim().split(RegExp(r'\s+')).join(' ').toLowerCase();

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

/// Unescape backslash-escaped ASCII punctuation.
String _unescapeMd(String s) {
  final out = StringBuffer();
  var i = 0;
  while (i < s.length) {
    if (s[i] == r'\' && i + 1 < s.length && _asciiPunct.contains(s[i + 1])) {
      out.write(s[i + 1]);
      i += 2;
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
    final buf = StringBuffer();
    while (j < n && src[j] != close) {
      if (src[j] == r'\' && j + 1 < n) {
        buf.write(src[j + 1]);
        j += 2;
        continue;
      }
      buf.write(src[j]);
      j++;
    }
    if (j >= n) return null;
    title = buf.toString();
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
  if (RegExp(r'^[A-Za-z][A-Za-z0-9+.-]*:.').hasMatch(inner)) return inner;
  if (RegExp(r'^[^@\s]+@[^@\s]+\.[^@\s.]+$').hasMatch(inner)) {
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

  void addLink(String label, String href, String? title) {
    final start = out.length;
    final parsed = parseInline(label, defs: defs);
    out.write(parsed.text);
    for (final m in parsed.marks) {
      marks.add(m.shifted(start));
    }
    marks.add(Mark(start, out.length, 'link', href: href, title: title));
  }

  while (i < src.length) {
    // Backslash escape: `\*` is a literal `*` (any ASCII punctuation).
    if (src[i] == r'\' &&
        i + 1 < src.length &&
        _asciiPunct.contains(src[i + 1])) {
      out.write(src[i + 1]);
      i += 2;
      continue;
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
    }
    // Links: [text](dest "title") | [text][label] | [text][] | [shortcut]
    // — unless preceded by `!` (an image; not an inline mark).
    if (src[i] == '[' && !(i > 0 && src[i - 1] == '!')) {
      final close = matchingBracket(src, i);
      if (close > i + 1) {
        final label = src.substring(i + 1, close);
        if (close + 1 < src.length && src[close + 1] == '(') {
          final suffix = parseLinkSuffix(src, close + 2);
          if (suffix != null) {
            addLink(label, suffix.dest, suffix.title);
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
            addLink(label, def.dest, def.title);
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
      final f = _flanking(c, i == 0 ? null : src[i - 1],
          j < src.length ? src[j] : null);
      delims.add(_Delim(c, out.length, count, f.open, f.close));
      out.write(c * count);
      i = j;
      continue;
    }
    if (src.startsWith('~~', i)) {
      final end = _indexOfUnescaped(src, '~~', i + 2);
      if (end > i + 1) {
        addInner(src.substring(i + 2, end), 'strike');
        i = end + 2;
        continue;
      }
    }
    if (src[i] == '`') {
      final end = src.indexOf('`', i + 1);
      if (end > i) {
        final start = out.length;
        out.write(src.substring(i + 1, end));
        marks.add(Mark(start, out.length, 'code'));
        i = end + 1;
        continue;
      }
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
  return _asciiPunct.contains(c) ||
      (c.codeUnitAt(0) >= 0x2010 && c.codeUnitAt(0) <= 0x2027);
}

({bool open, bool close}) _flanking(String c, String? prev, String? next) {
  final prevWs = prev == null || prev.trim().isEmpty;
  final nextWs = next == null || next.trim().isEmpty;
  final prevPunct = _isMdPunct(prev);
  final nextPunct = _isMdPunct(next);
  final left = !nextWs && (!nextPunct || prevWs || prevPunct);
  final right = !prevWs && (!prevPunct || nextWs || nextPunct);
  if (c == '_') {
    return (
      open: left && (!right || prevPunct),
      close: right && (!left || nextPunct),
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
/// `1. x`, `---`) must escape its leader or it changes kind on re-parse.
/// Mirrors the Rust engine.
String escapeBlockLeader(String line) {
  final dividerLike =
      line.length >= 3 && line.split('').every((c) => c == '-');
  if (line.startsWith('- ') ||
      line.startsWith('+ ') ||
      line.startsWith('> ') ||
      dividerLike ||
      RegExp(r'^#{1,6} ').hasMatch(line)) {
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
    if (r'\*_`~[]<'.contains(c)) out.write(r'\');
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
    out.write(escapeInline(text.substring(pos, ps)));
    if (pick.type == 'code') {
      // Code spans are literal — no escaping, no nested marks.
      out.write('`${text.substring(ps, pe)}`');
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
      default:
        out.write(body);
    }
    pos = pe;
  }
  return out.toString();
}
