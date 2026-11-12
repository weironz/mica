import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  for (final tokens in [MicaTokens.light, MicaTokens.dark_]) {
    testWidgets('stock list text resolves readable ink dark=${tokens.dark}', (
      t,
    ) async {
      await t.pumpWidget(
        MaterialApp(
          theme: tokens.toMaterialTheme(),
          home: const Scaffold(
            body: ListTile(
              title: Text('Row label'),
              subtitle: Text('Row detail'),
            ),
          ),
        ),
      );
      for (final label in ['Row label', 'Row detail']) {
        final rich = find.descendant(
          of: find.text(label),
          matching: find.byType(RichText),
        );
        final paragraph = t.renderObject<RenderParagraph>(rich);
        expect(paragraph.text.style!.color, tokens.text.primary);
      }
      expect(t.takeException(), isNull);
    });
  }
}
