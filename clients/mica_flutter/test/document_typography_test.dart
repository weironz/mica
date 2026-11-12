import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cjk_fonts.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  tearDown(() => debugDefaultTargetPlatformOverride = null);
  test(
    'widgets and canvas share the default Western family on every platform',
    () {
      final node = EditorNode(id: 'p', kind: 'paragraph', text: 'Mica 0123');
      for (final platform in TargetPlatform.values) {
        debugDefaultTargetPlatformOverride = platform;
        final theme = MicaTokens.light.toMaterialTheme();
        final style = const EditorAppearance().applyTo(
          EditorTheme.styleFor(node, MicaTokens.light),
          isCode: false,
        );
        expect(
          theme.textTheme.bodyMedium!.fontFamily,
          uiFontFamily,
          reason: '$platform UI',
        );
        expect(style.fontFamily, uiFontFamily, reason: '$platform canvas');
      }
    },
  );
  test(
    'document rows round after scaling, including small scaled headings',
    () {
      const rows = [
        ('paragraph', 0, 16.0, 26.0, FontWeight.w400),
        ('heading', 1, 30.0, 39.0, FontWeight.w600),
        ('heading', 2, 24.0, 32.0, FontWeight.w600),
        ('heading', 3, 20.0, 28.0, FontWeight.w600),
        ('heading', 4, 18.0, 26.0, FontWeight.w600),
        ('heading', 5, 16.0, 24.0, FontWeight.w600),
        ('heading', 6, 14.0, 22.0, FontWeight.w500),
        ('quote', 0, 16.0, 26.0, FontWeight.w400),
        ('footnote_def', 0, 13.0, 20.0, FontWeight.w400),
        ('code_block', 0, 14.0, 22.0, FontWeight.w400),
        ('math_block', 0, 14.0, 22.0, FontWeight.w400),
      ];
      for (final (kind, level, size, height, weight) in rows) {
        final node = EditorNode(
          id: 'b',
          kind: kind,
          text: '文字 Agent',
          data: {'level': level},
        );
        for (final scale in [0.85, 1.0, 1.1, 1.25, 1.4]) {
          final appearance = EditorAppearance(fontScale: scale);
          final style = appearance.applyTo(
            EditorTheme.styleFor(node, MicaTokens.light),
            isCode: kind == 'code_block',
          );
          expect(style.fontSize, size * scale, reason: '$kind/$level size');
          expect(
            style.fontSize! * style.height!,
            closeTo((height * scale).round(), 1e-9),
            reason: '$kind/$level scaled row',
          );
          final expectedWeight =
              size * scale < 16 && weight.value >= FontWeight.w600.value
              ? FontWeight.w500
              : weight;
          expect(
            style.fontWeight ?? FontWeight.w400,
            expectedWeight,
            reason: '$kind/$level weight',
          );
        }
      }
    },
  );
  test('user prose family wins while code remains monospace', () {
    const appearance = EditorAppearance(fontFamily: 'Georgia');
    final prose = EditorNode(id: 'p', kind: 'paragraph', text: 'text');
    final code = EditorNode(id: 'c', kind: 'code_block', text: 'code');
    expect(
      appearance
          .applyTo(EditorTheme.styleFor(prose, MicaTokens.light), isCode: false)
          .fontFamily,
      'Georgia',
    );
    expect(
      appearance
          .applyTo(EditorTheme.styleFor(code, MicaTokens.light), isCode: true)
          .fontFamily,
      kMonoFont,
    );
  });
}
