import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/mermaid_theme.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

/// A diagram is the one thing on the page drawn by somebody else's engine, so it
/// is also the easiest thing to leave behind on a theme switch — a light diagram
/// sitting on a dark page like a sticker. What these pin is that the colours
/// handed to merman come from the app's own palette and actually differ
/// between the two.
void main() {
  group('cssHex', () {
    test('emits #RRGGBB, which is what SVG attributes take', () {
      expect(cssHex(const Color(0xFF2563EB)), '#2563eb');
      expect(cssHex(const Color(0xFFFFFFFF)), '#ffffff');
      expect(cssHex(const Color(0xFF000000)), '#000000');
    });

    test('pads single-digit channels instead of shortening them', () {
      // '#f0509' is not a colour. Dropping the pad silently shifts every
      // channel, which reads as "the diagram engine is broken".
      expect(cssHex(const Color(0xFF0F0509)), '#0f0509');
    });

    test('drops alpha rather than emitting #RRGGBBAA', () {
      // These land in fill/stroke, where a translucent node would let the page
      // show through and stop reading as a node at all.
      expect(cssHex(const Color(0x802563EB)), '#2563eb');
    });
  });

  group('the palette handed to merman is the app palette', () {
    test('light maps the roles a diagram needs', () {
      const t = MicaTokens.light;
      final theme = mermaidThemeFor(t);

      expect(theme.dark, isFalse);
      expect(theme.canvas, cssHex(t.surface.base));
      expect(theme.surface, cssHex(t.surface.raised));
      expect(theme.text, cssHex(t.text.primary));
      expect(theme.line, cssHex(t.border.strong));
      expect(theme.error, cssHex(t.status.danger));
    });

    test('dark is a different palette, not the same one with a flag', () {
      // The flag alone would let every colour stay light while merman merely
      // *believed* it was drawing dark.
      final light = mermaidThemeFor(MicaTokens.light);
      final dark = mermaidThemeFor(MicaTokens.dark_);

      expect(dark.dark, isTrue);
      expect(dark.canvas, isNot(light.canvas));
      expect(dark.surface, isNot(light.surface));
      expect(dark.text, isNot(light.text));
      expect(dark.line, isNot(light.line));
    });
  });
}
