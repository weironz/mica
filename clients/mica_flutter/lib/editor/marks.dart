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

/// Parse inline Markdown (`**b**`, `*i*`/`_i_`, `` `c` ``, `~~s~~`,
/// `[t](url)`) into clean text plus marks.
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
      final end = src.indexOf('**', i + 2);
      if (end > i + 1) {
        addInner(src.substring(i + 2, end), 'bold');
        i = end + 2;
        continue;
      }
    }
    if (src.startsWith('~~', i)) {
      final end = src.indexOf('~~', i + 2);
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
      final end = src.indexOf(ch, i + 1);
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

/// Serialize text + marks back to inline Markdown (used for copy/export).
String inlineToMarkdown(String text, List<Mark> marks) {
  if (marks.isEmpty) return text;
  String wrap(String s, String marker) => '$marker$s$marker';
  // Apply non-link marks by wrapping their (assumed non-crossing) ranges,
  // innermost first; links wrap as [text](href).
  final ordered = [...marks]..sort((a, b) => (b.end - b.start) - (a.end - a.start));
  // Work on a list of (char, set<markers>) — simpler: rebuild by segments.
  final len = text.length;
  final points = <int>{0, len};
  for (final m in marks) {
    points
      ..add(m.start.clamp(0, len))
      ..add(m.end.clamp(0, len));
  }
  final sorted = points.toList()..sort();
  final buffer = StringBuffer();
  for (var i = 0; i < sorted.length - 1; i++) {
    final a = sorted[i];
    final b = sorted[i + 1];
    if (a >= b) continue;
    var seg = text.substring(a, b);
    String? href;
    var hasLink = false;
    for (final m in ordered) {
      if (m.start <= a && m.end >= b) {
        switch (m.type) {
          case 'code':
            seg = wrap(seg, '`');
          case 'bold':
            seg = wrap(seg, '**');
          case 'italic':
            seg = wrap(seg, '*');
          case 'strike':
            seg = wrap(seg, '~~');
          case 'link':
            hasLink = true;
            href = m.href;
        }
      }
    }
    if (hasLink) seg = '[$seg]($href)';
    buffer.write(seg);
  }
  return buffer.toString();
}
