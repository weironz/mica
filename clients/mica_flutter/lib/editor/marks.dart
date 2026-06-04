import 'package:flutter/widgets.dart';

/// Inline rich-text marks stored additively over a block's plain text (in
/// `data.marks`). The text stays clean; a mark is a `[start, end)` range with a
/// type (and an href for links). See docs/editor-engine.md.
class Mark {
  Mark(this.start, this.end, this.type, {this.href});

  final int start;
  final int end;
  final String type; // bold | italic | code | strike | link
  final String? href;

  Mark shifted(int delta) => Mark(start + delta, end + delta, type, href: href);

  Map<String, dynamic> toJson() => {
    'start': start,
    'end': end,
    'type': type,
    if (href != null) 'href': href,
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
        marks.add(Mark(start, end, type, href: m['href'] as String?));
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
({String text, List<Mark> marks}) parseInline(String src) {
  final out = StringBuffer();
  final marks = <Mark>[];
  var i = 0;

  void addInner(String inner, String type, {String? href}) {
    final start = out.length;
    final parsed = parseInline(inner);
    out.write(parsed.text);
    for (final m in parsed.marks) {
      marks.add(m.shifted(start));
    }
    marks.add(Mark(start, out.length, type, href: href));
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
    // A `[..](..)` is a link — unless preceded by `!`, which makes it an image
    // (`![alt](url)`); images aren't inline marks, so leave them as literal text.
    final link = (i > 0 && src[i - 1] == '!')
        ? null
        : RegExp(r'\[([^\]]+)\]\(([^)\s]+)\)').matchAsPrefix(src, i);
    if (link != null) {
      final start = out.length;
      out.write(link.group(1)!);
      marks.add(Mark(start, out.length, 'link', href: link.group(2)));
      i = link.end;
      continue;
    }
    if (src.startsWith('**', i)) {
      final end = _indexOfUnescaped(src, '**', i + 2);
      if (end > i + 1) {
        addInner(src.substring(i + 2, end), 'bold');
        i = end + 2;
        continue;
      }
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
    if (src[i] == '*' || src[i] == '_') {
      final ch = src[i];
      final end = _indexOfUnescaped(src, ch, i + 1);
      if (end > i + 1) {
        addInner(src.substring(i + 1, end), 'italic');
        i = end + 1;
        continue;
      }
    }
    out.write(src[i]);
    i++;
  }
  return (text: out.toString(), marks: _normalize(marks));
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
        if (href == plain || href == 'mailto:$plain') {
          out.write('<$plain>');
        } else {
          out.write('[$body]($href)');
        }
      default:
        out.write(body);
    }
    pos = pe;
  }
  return out.toString();
}
