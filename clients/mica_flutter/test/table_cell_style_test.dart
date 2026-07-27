import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/render.dart';

/// The bug: clicking a cell in the header row un-bolded it. Four places needed
/// this style — the painter, the inline editor's overlay field, and two throwaway
/// painters that measure the cell for the caret and the drag selection — and each
/// had its own `const TextStyle(fontSize: 15, height: 1.4)`, which dropped both
/// the header weight and the user's font settings.
///
/// So what is worth testing is not "is it 15px" but "does the editor get the same
/// thing the painter gets".
void main() {
  const plain = EditorAppearance();

  test('a header cell is bolder than a body cell', () {
    final header = RenderDocument.tableCellStyle(plain, isHeader: true);
    final body = RenderDocument.tableCellStyle(plain, isHeader: false);

    expect(header.fontWeight, FontWeight.w600);
    expect(body.fontWeight, FontWeight.w400);
  });

  test('the user font scale reaches the cell', () {
    // Without this, clicking into any cell snapped it back to 15px for anyone who
    // had changed the editor font size — and the measuring painters were off by
    // the same amount, so the caret landed in the wrong place.
    final normal = RenderDocument.tableCellStyle(plain, isHeader: false);
    final scaled = RenderDocument.tableCellStyle(
      const EditorAppearance(fontScale: 1.4),
      isHeader: false,
    );

    expect(normal.fontSize, 15);
    expect(scaled.fontSize, closeTo(21, 0.001));
  });

  test('the font family reaches the cell, and a CJK fallback is attached', () {
    final styled = RenderDocument.tableCellStyle(
      const EditorAppearance(fontFamily: 'Serif'),
      isHeader: false,
    );

    expect(styled.fontFamily, 'Serif');
    expect(
      styled.fontFamilyFallback,
      isNotEmpty,
      reason: 'CJK text in a cell must not fall back to notdef boxes',
    );
  });

  test('scale and header weight compose — a scaled header is both', () {
    final style = RenderDocument.tableCellStyle(
      const EditorAppearance(fontScale: 1.2),
      isHeader: true,
    );

    expect(style.fontWeight, FontWeight.w600);
    expect(style.fontSize, closeTo(18, 0.001));
  });

  test('everything except the weight matches between header and body', () {
    // The header row differs in exactly ONE way. If a future edit makes the editor
    // build its own style again, this is the shape it would have to match.
    final header = RenderDocument.tableCellStyle(plain, isHeader: true);
    final body = RenderDocument.tableCellStyle(plain, isHeader: false);

    expect(header.copyWith(fontWeight: body.fontWeight), body);
  });
}
