import 'package:flutter/widgets.dart';
import 'model.dart' show kMonoFont;
import '../ui/theme_tokens.dart';

import '../markdown/mark.dart';

export '../markdown/mark.dart';

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

/// Clip [marks] to the absolute range `[from, to)` and rebase to 0 — the model
/// half of what the old HTML writer did inline. Both the block selection
/// (`selectionBlocks`) and the table-cell editor clip through here, so a mark
/// partially inside the selection survives on its overlap (a link cut in half
/// keeps its href), and a zero-width link that lands ON the boundary survives
/// (`marksFromData` deliberately keeps those). Atomic blocks call it with the
/// whole-block range, which returns the marks unchanged.
List<Mark> clipMarks(List<Mark> marks, int from, int to) {
  final clipped = <Mark>[];
  for (final mark in marks) {
    final start = mark.start.clamp(from, to);
    final end = mark.end.clamp(from, to);
    final empty = end == start;
    if (end > start || (empty && mark.type == 'link' && mark.start >= from && mark.end <= to)) {
      clipped.add(Mark(start - from, end - from, mark.type, href: mark.href, title: mark.title));
    }
  }
  return clipped;
}

/// The math run a caret at [caret] sits inside, or null — the core of the
/// "a formula is one atom" rule: selections snap OUT of a run (see
/// [snapOutOfMathRun]) and arrow keys step over the whole run (editor.dart's
/// horizontal move).
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
    if (m.start < from) result.add(m.withRange(m.start, from));
    if (m.end > to) result.add(m.withRange(to, m.end));
  }
  if (add) result.add(Mark(from, to, type, href: href));
  return _normalize(result);
}

/// Shift mark offsets for a text edit that replaced `[editStart, editOldEnd)`
/// with `delta = newLen - oldLen` net characters.
///
/// [pinAtStart] (the typing policy) keeps a mark's start put when text is
/// inserted exactly at it, so typing at a mark's leading edge extends the mark.
/// Pasting unmarked text uses `pinAtStart: false`: a mark that starts at the
/// insertion point must move with its own characters, not swallow the paste.
List<Mark> shiftMarks(
  List<Mark> marks,
  int editStart,
  int editOldEnd,
  int delta,
  int newLen, {
  bool pinAtStart = true,
}) {
  int move(int x) {
    if (pinAtStart) {
      if (x <= editStart) return x;
    } else {
      if (x >= editOldEnd) return x + delta;
      if (x > editStart) return editStart;
      return x;
    }
    if (x >= editOldEnd) return x + delta;
    return editStart; // endpoint inside the replaced region collapses
  }

  final result = <Mark>[];
  for (final m in marks) {
    final s = move(m.start).clamp(0, newLen);
    final e = move(m.end).clamp(0, newLen);
    if (e > s) result.add(m.withRange(s, e));
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
      if (end > m.start) before.add(m.withRange(m.start, end));
    }
    if (m.end > at) {
      final start = m.start > at ? m.start : at;
      if (m.end > start) {
        after.add(m.withRange(start - at, m.end - at));
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
        // A different link title is a DIFFERENT link: fusing them would keep
        // one title and silently discard the other (issue #150).
        out.last.title == m.title &&
        out.last.end >= m.start) {
      final prev = out.removeLast();
      out.add(prev.withRange(prev.start, m.end > prev.end ? m.end : prev.end));
    } else {
      out.add(m);
    }
  }
  return out;
}

/// Value equality for two mark lists, ignoring input order: the canonical
/// order is `(start, end, type, href, title)`.
///
/// [Mark] deliberately has no `operator ==` — parts of the editor rely on
/// reference identity for the marks they build, so changing its semantics was
/// never evaluated. Comparing the structured fields here is enough for the
/// model that needs it (table cells).
bool marksEqual(List<Mark> a, List<Mark> b) {
  if (identical(a, b)) return true;
  if (a.length != b.length) return false;
  if (a.isEmpty) return true;
  final x = [...a]..sort(_compareMarks);
  final y = [...b]..sort(_compareMarks);
  for (var i = 0; i < x.length; i++) {
    final m = x[i], n = y[i];
    if (m.start != n.start ||
        m.end != n.end ||
        m.type != n.type ||
        m.href != n.href ||
        m.title != n.title) {
      return false;
    }
  }
  return true;
}

int _compareMarks(Mark a, Mark b) {
  var c = a.start.compareTo(b.start);
  if (c != 0) return c;
  c = a.end.compareTo(b.end);
  if (c != 0) return c;
  c = a.type.compareTo(b.type);
  if (c != 0) return c;
  c = _compareNullable(a.href, b.href);
  if (c != 0) return c;
  return _compareNullable(a.title, b.title);
}

int _compareNullable(String? a, String? b) {
  if (a == b) return 0;
  if (a == null) return -1;
  if (b == null) return 1;
  return a.compareTo(b);
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
