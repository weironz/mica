import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  test('remaining direct blue roles have explicit reviewed purposes', () {
    final expected = jsonDecode(
      File('test/fixtures/blue_accent_uses.json').readAsStringSync(),
    );
    final actual = <String, int>{};
    for (final f in Directory(
      'lib',
    ).listSync(recursive: true).whereType<File>()) {
      if (!f.path.endsWith('.dart')) continue;
      for (final line in f.readAsStringSync().split('\n')) {
        if (line.trimLeft().startsWith('//')) continue;
        for (final m in RegExp(
          r'\b(?:accent\.(?:primary|wash|hover)|colorScheme\.primary|secondaryContainer|editor\.caret)\b',
        ).allMatches(line)) {
          final k = '${f.path.replaceAll('\\', '/')}#${m[0]}';
          actual.update(k, (n) => n + 1, ifAbsent: () => 1);
        }
      }
    }
    expect(
      actual,
      expected,
      reason:
          'Selections and decorative controls must use neutral roles; new blue uses need a semantic purpose.',
    );
  });
  test(
    'framework defaults keep secondary controls neutral and selection blue',
    () {
      for (final t in [MicaTokens.light, MicaTokens.dark_]) {
        final theme = t.toMaterialTheme();
        expect(theme.sliderTheme.thumbColor, t.text.primary);
        expect(
          theme.radioTheme.fillColor!.resolve({WidgetState.selected}),
          t.text.primary,
        );
        expect(
          theme.textButtonTheme.style!.foregroundColor!.resolve({}),
          t.text.primary,
        );
        expect(theme.textSelectionTheme.cursorColor, t.editor.caret);
        expect(theme.textSelectionTheme.selectionColor, t.editor.selection);
        expect(theme.colorScheme.primary, t.accent.primary);
      }
    },
  );
}
