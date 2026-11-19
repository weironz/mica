import 'dart:ui' as ui;

import 'package:flutter/widgets.dart' hide TableCell;

import '../ui/theme_tokens.dart';
import 'marks.dart';
import 'table.dart';

/// Inline code uses a slightly smaller font size than surrounding body text
/// so monospace glyphs visually align with CJK and Latin prose (gh#189).
const double kInlineCodeFontScale = 0.88;

/// Horizontal padding painted inside the inline-code rounded pill.
const double kInlineCodePillPadH = 3.5;

/// Vertical padding painted inside the inline-code rounded pill.
const double kInlineCodePillPadV = 1.5;

/// Extra breathing room outside the pill edge when adjacent to normal text.
const double kInlineCodeOuterGap = 2.0;

/// Corner radius for inline-code pills (shared by paragraphs and table cells).
const double kInlineCodeRadius = 4.0;

/// A zero-character spacer inserted at an inline-code boundary in a display
/// [TextPainter] so the rounded pill never crowds or overlaps adjacent glyphs.
class InlineCodeSpacer {
  const InlineCodeSpacer({
    required this.docOffset,
    required this.width,
    required this.attachAfter,
  });

  /// Boundary offset in document text coordinates.
  final int docOffset;

  /// Logical pixel width reserved in the line layout.
  final double width;

  /// True for a leading spacer (sits immediately before the code run at
  /// [docOffset], so doc offsets >= [docOffset] fall to its right).
  /// False for a trailing spacer (sits immediately after the code run at
  /// [docOffset], so only doc offsets > [docOffset] fall to its right).
  final bool attachAfter;
}

/// Extract sorted, merged, clamped `[start, end)` ranges of `code` marks.
List<({int start, int end})> mergedInlineCodeRanges(
  int textLength,
  List<Mark> marks,
) {
  final raw = <({int start, int end})>[];
  for (final m in marks) {
    if (m.type != 'code') continue;
    final s = m.start.clamp(0, textLength);
    final e = m.end.clamp(0, textLength);
    if (e > s) raw.add((start: s, end: e));
  }
  if (raw.length <= 1) return raw;
  raw.sort((a, b) {
    final c = a.start.compareTo(b.start);
    return c != 0 ? c : a.end.compareTo(b.end);
  });
  final merged = <({int start, int end})>[raw.first];
  for (var i = 1; i < raw.length; i++) {
    final prev = merged.last;
    final cur = raw[i];
    if (cur.start <= prev.end) {
      merged[merged.length - 1] = (
        start: prev.start,
        end: cur.end > prev.end ? cur.end : prev.end,
      );
    } else {
      merged.add(cur);
    }
  }
  return merged;
}

/// Leading and trailing spacers for every inline-code run in [text].
///
/// Adjacent to text, reserve pill padding + outer breathing gap so the pill
/// never touches neighboring glyphs; at the start/end of the block, reserve
/// only the pill's own horizontal padding so the pill aligns with the edge.
List<InlineCodeSpacer> inlineCodeSpacers(String text, List<Mark> marks) {
  final ranges = mergedInlineCodeRanges(text.length, marks);
  if (ranges.isEmpty) return const [];
  final out = <InlineCodeSpacer>[];
  for (final r in ranges) {
    out.add(
      InlineCodeSpacer(
        docOffset: r.start,
        width: r.start > 0
            ? kInlineCodePillPadH + kInlineCodeOuterGap
            : kInlineCodePillPadH,
        attachAfter: true,
      ),
    );
    out.add(
      InlineCodeSpacer(
        docOffset: r.end,
        width: r.end < text.length
            ? kInlineCodePillPadH + kInlineCodeOuterGap
            : kInlineCodePillPadH,
        attachAfter: false,
      ),
    );
  }
  return out;
}

/// Shared inline-code pill painter used by both paragraphs and table cells so
/// both always render the same rounded pill geometry and color (gh#188, gh#189).
void paintInlineCodePills(
  Canvas canvas,
  Offset origin,
  TextPainter painter,
  Iterable<({int start, int end})> painterRanges,
  Color bgColor,
) {
  final paint = Paint()..color = bgColor;
  for (final r in painterRanges) {
    if (r.end <= r.start) continue;
    final boxes = painter.getBoxesForSelection(
      TextSelection(baseOffset: r.start, extentOffset: r.end),
      boxHeightStyle: ui.BoxHeightStyle.tight,
    );
    for (final b in boxes) {
      final rect = b.toRect().shift(origin);
      final chip = Rect.fromLTRB(
        rect.left - kInlineCodePillPadH,
        rect.top - kInlineCodePillPadV,
        rect.right + kInlineCodePillPadH,
        rect.bottom + kInlineCodePillPadV,
      );
      canvas.drawRRect(
        RRect.fromRectAndRadius(chip, const Radius.circular(kInlineCodeRadius)),
        paint,
      );
    }
  }
}

/// Build the display span, placeholder dimensions, and painter-space code
/// ranges for a read-mode table cell (gh#188, gh#189).
({
  TextSpan span,
  List<PlaceholderDimensions> dims,
  List<({int start, int end})> codeRanges,
})
buildTableCellDisplay(TableCell cell, TextStyle base, MicaTokens tokens) {
  if (cell.isEmpty) {
    return (
      span: TextSpan(text: ' ', style: base),
      dims: const [],
      codeRanges: const [],
    );
  }
  final ranges = mergedInlineCodeRanges(cell.text.length, cell.marks);
  if (ranges.isEmpty) {
    return (
      span: buildMarkedSpan(cell.text, cell.marks, base, tokens),
      dims: const [],
      codeRanges: const [],
    );
  }
  final spacers = inlineCodeSpacers(cell.text, cell.marks);
  final children = <InlineSpan>[];
  final dims = <PlaceholderDimensions>[];
  final codeRanges = <({int start, int end})>[];

  void emitText(int from, int to) {
    if (to <= from) return;
    final localMarks = <Mark>[
      for (final m in cell.marks)
        if (m.end > from && m.start < to)
          Mark(
            (m.start - from).clamp(0, to - from),
            (m.end - from).clamp(0, to - from),
            m.type,
            href: m.href,
            title: m.title,
          ),
    ];
    children.add(
      buildMarkedSpan(cell.text.substring(from, to), localMarks, base, tokens),
    );
  }

  void emitSpacer(double width) {
    children.add(
      const WidgetSpan(
        alignment: ui.PlaceholderAlignment.baseline,
        baseline: TextBaseline.alphabetic,
        child: SizedBox.shrink(),
      ),
    );
    dims.add(
      PlaceholderDimensions(
        size: Size(width, 1),
        alignment: ui.PlaceholderAlignment.baseline,
        baseline: TextBaseline.alphabetic,
        baselineOffset: 1,
      ),
    );
  }

  var cursor = 0;
  var painterPos = 0;
  for (var i = 0; i < ranges.length; i++) {
    final r = ranges[i];
    final leftSpacer = spacers[i * 2];
    final rightSpacer = spacers[i * 2 + 1];
    emitText(cursor, r.start);
    painterPos += r.start - cursor;

    emitSpacer(leftSpacer.width);
    painterPos += 1;

    final pStart = painterPos;
    emitText(r.start, r.end);
    painterPos += r.end - r.start;
    codeRanges.add((start: pStart, end: painterPos));

    emitSpacer(rightSpacer.width);
    painterPos += 1;

    cursor = r.end;
  }
  emitText(cursor, cell.text.length);

  return (
    span: TextSpan(style: base, children: children),
    dims: dims,
    codeRanges: codeRanges,
  );
}

/// Build an unlaid-out [TextPainter] (with spacer placeholder dimensions set)
/// and painter-space inline-code ranges for one read-mode table cell.
({TextPainter painter, List<({int start, int end})> codeRanges})
buildTableCellPainter(
  TableCell cell,
  TextStyle base,
  MicaTokens tokens,
  TextAlign align,
) {
  final display = buildTableCellDisplay(cell, base, tokens);
  final painter = TextPainter(
    text: display.span,
    textAlign: align,
    textDirection: TextDirection.ltr,
  );
  if (display.dims.isNotEmpty) painter.setPlaceholderDimensions(display.dims);
  return (painter: painter, codeRanges: display.codeRanges);
}
