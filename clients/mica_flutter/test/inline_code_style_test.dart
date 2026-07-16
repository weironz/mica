import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart' show kMonoFont;

// Inline code moved from "red mono text + tight faint-red box" to "neutral mono
// ink + a rounded pill drawn in the render layer" (_paintInlineCode). The span
// now carries the mono font + calm ink and NO tight backgroundColor.
void main() {
  TextStyle codeStyle() {
    final span = buildMarkedSpan(
      'code',
      [Mark(0, 4, 'code')],
      const TextStyle(color: Color(0xFF24292F), fontSize: 16),
    );
    return (span.children!.first as TextSpan).style!;
  }

  test('inline code: mono font, neutral ink, no tight backgroundColor', () {
    final s = codeStyle();
    expect(s.fontFamily, kMonoFont);
    expect(s.color, const Color(0xFF334155));
    expect(s.backgroundColor, isNull, reason: 'the pill is drawn by the render layer now');
  });

  test('bold / italic / strike still map to their styles', () {
    const base = TextStyle(color: Color(0xFF24292F));
    TextStyle only(String type) =>
        (buildMarkedSpan('x', [Mark(0, 1, type)], base).children!.first
                as TextSpan)
            .style!;
    expect(only('bold').fontWeight, FontWeight.w600);
    expect(only('italic').fontStyle, FontStyle.italic);
    expect(only('strike').decoration, TextDecoration.lineThrough);
  });

  _boldTests();
}

// Bold: does applying it change the SIZE, or only the weight? Asked directly
// because 加粗 CJK reads noticeably heavier on screen and "the font got bigger"
// is the natural suspicion. It does not: the answer is fake-bold. We bundle
// only NotoSansSC-Regular (no bold face), so for CJK the engine synthesizes
// weight by widening strokes, which looks heavier and slightly wider without
// any fontSize change. Shipping a real bold CJK face would cost ~10MB in the
// web bundle — a deliberate trade, not an oversight.
void _boldTests() {
  TextStyle styleOf(String type) {
    final span = buildMarkedSpan(
      'text',
      [Mark(0, 4, type)],
      const TextStyle(color: Color(0xFF24292F), fontSize: 16),
    );
    return (span.children!.first as TextSpan).style!;
  }

  test('bold changes weight only — never the font size', () {
    final s = styleOf('bold');
    expect(s.fontWeight, FontWeight.w600);
    expect(s.fontSize, 16, reason: 'bold must not scale the text');
    expect(s.fontFamily, isNull, reason: 'bold must not swap the family');
  });

  test('no inline mark rescales the text', () {
    for (final type in ['bold', 'italic', 'strike', 'code']) {
      expect(styleOf(type).fontSize, 16, reason: '$type changed fontSize');
    }
  });
}
