import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';

/// A changed baseline needs a reviewed purpose, not merely a new count.
/// Ghost is restricted to hints, disabled controls and a breadcrumb separator.
void main() {
  test('ghost has only the reviewed non-informational consumers', () {
    const allowed = {
      'lib/ui/theme_tokens.dart': 5, // Hint + disabled theme and three buttons.
      'lib/ui/page_title_field.dart': 1, // Title hint.
      'lib/editor/property_panel.dart': 3, // Value, tag and property-key hints.
      'lib/ui/emoji_picker.dart': 1, // Search hint.
      'lib/ui/shell_widgets.dart': 1, // Decorative breadcrumb chevron.
    };
    final actual = <String, int>{};
    for (final file in Directory(
      'lib',
    ).listSync(recursive: true).whereType<File>()) {
      if (!file.path.endsWith('.dart')) continue;
      final count = RegExp(
        r'\btext\.ghost\b',
      ).allMatches(file.readAsStringSync()).length;
      if (count != 0) actual[file.path.replaceAll('\\', '/')] = count;
    }
    expect(
      actual,
      allowed,
      reason:
          'Every new ghost use needs an explicit non-informational purpose.',
    );
  });

  test('components introduce no unreviewed literal colors', () {
    final expected = jsonDecode(
      File('test/fixtures/ui_color_literals.json').readAsStringSync(),
    );
    final actual = <String, Map<String, int>>{};
    for (final file in Directory(
      'lib',
    ).listSync(recursive: true).whereType<File>()) {
      final path = file.path.replaceAll('\\', '/');
      if (!path.endsWith('.dart') || path == 'lib/ui/theme_tokens.dart') {
        continue;
      }
      final colors = <String, int>{};
      for (final match in RegExp(
        r'Color\(0x([0-9A-Fa-f]+)\)',
      ).allMatches(file.readAsStringSync())) {
        final value = match[1]!.toUpperCase();
        colors.update(value, (n) => n + 1, ifAbsent: () => 1);
      }
      if (colors.isNotEmpty) actual[path] = colors;
    }
    expect(
      actual,
      expected,
      reason:
          'New component colors must read semantic tokens. Existing brand/status/highlight colors remain frozen.',
    );
  });
}
