import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/inline_code_layout.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart' show kMonoFont;
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

// Inline code moved from "red mono text + tight faint-red box" to "neutral mono
// ink + a rounded pill drawn in the render layer" (_paintInlineCode). The span
// now carries the mono font + calm ink and NO tight backgroundColor.
void main() {
  TextStyle codeStyle() {
    final span = buildMarkedSpan(
      'code',
      [Mark(0, 4, 'code')],
      const TextStyle(color: Color(0xFF24292F), fontSize: 16),
      MicaTokens.light,
    );
    return (span.children!.first as TextSpan).style!;
  }

  test('inline code: mono font, neutral ink, no tight backgroundColor', () {
    final s = codeStyle();
    expect(s.fontFamily, kMonoFont);
    // The ROLE, not the hex. This used to pin #334155, which is still what the
    // light palette resolves to — but pinning the byte would mean the dark
    // palette could not exist without editing the assertion.
    expect(s.color, MicaTokens.light.editor.inlineCodeInk);
    expect(
      s.backgroundColor,
      isNull,
      reason: 'the pill is drawn by the render layer now',
    );
  });

  test('bold / italic / strike still map to their styles', () {
    const base = TextStyle(color: Color(0xFF24292F));
    TextStyle only(String type) =>
        (buildMarkedSpan(
                  'x',
                  [Mark(0, 1, type)],
                  base,
                  MicaTokens.light,
                ).children!.first
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
      MicaTokens.light,
    );
    return (span.children!.first as TextSpan).style!;
  }

  test('bold changes weight only — never the font size', () {
    final s = styleOf('bold');
    expect(s.fontWeight, FontWeight.w600);
    expect(s.fontSize, 16, reason: 'bold must not scale the text');
    expect(s.fontFamily, isNull, reason: 'bold must not swap the family');
  });

  test('prose marks do not rescale text; inline code scales slightly smaller', () {
    for (final type in ['bold', 'italic', 'strike']) {
      expect(styleOf(type).fontSize, 16, reason: '$type changed fontSize');
    }
    expect(
      styleOf('code').fontSize,
      closeTo(16 * kInlineCodeFontScale, 1e-6),
      reason: 'inline code should be slightly smaller than body prose (gh#189)',
    );
  });

  test('inline code spacers reserve horizontal breathing room and map offsets cleanly', () {
    const text = '配置 INFISICAL_URL、INFISICAL_TOKEN 完成';
    final marks = [Mark(3, 16, 'code'), Mark(17, 32, 'code')];
    final spacers = inlineCodeSpacers(text, marks);
    expect(spacers, hasLength(4));
    for (final s in spacers) {
      expect(s.width, kInlineCodePillPadH + kInlineCodeOuterGap);
    }
    final fold = FoldPlan(const [], spacers);
    final folded = buildFoldedSpan(
      text,
      marks,
      const TextStyle(fontSize: 16),
      fold,
      MicaTokens.light,
    );
    final painter = TextPainter(
      text: folded.span,
      textDirection: TextDirection.ltr,
    )..setPlaceholderDimensions(folded.dims);
    painter.layout();
    // Plain text preserves every character once plus 4 U+FFFC spacers.
    expect(painter.plainText.replaceAll('\uFFFC', ''), text);
    expect(folded.dims, hasLength(4));
    // Every doc offset round-trips through docToPainter → painterToDoc.
    for (var doc = 0; doc <= text.length; doc++) {
      final p = fold.docToPainter(doc);
      expect(fold.painterToDoc(p), doc, reason: 'doc=$doc -> p=$p');
    }
    // Code range in painter space covers exact glyphs, excluding spacers.
    final p1Start = fold.docToPainter(3);
    final p1End = fold.docToPainter(16);
    expect(painter.plainText.substring(p1Start, p1End), 'INFISICAL_URL');
    final p2Start = fold.docToPainter(17);
    final p2End = fold.docToPainter(32);
    expect(painter.plainText.substring(p2Start, p2End), 'INFISICAL_TOKEN');
    painter.dispose();
  });
}
