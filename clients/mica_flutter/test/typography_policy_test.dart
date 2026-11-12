import 'dart:io';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'text sizes stay on the reviewed ladder, and heavy UI weights stay absent',
    () {
      final unexpected = <String>[];
      var h4Count = 0;
      const ladder = {11, 12, 13, 14, 16, 20, 24, 30};
      for (final file in Directory(
        'lib',
      ).listSync(recursive: true).whereType<File>()) {
        if (!file.path.endsWith('.dart')) continue;
        final path = file.path.replaceAll('\\', '/');
        final code = file.readAsStringSync().replaceAll(
          RegExp(r'//[^\n]*|/\*[\s\S]*?\*/'),
          '',
        );
        for (final match in RegExp(
          r'\bfontSize:\s*(\d+(?:\.\d+)?)\b',
        ).allMatches(code)) {
          final size = double.parse(match[1]!);
          if (ladder.contains(size)) continue;
          if (size == 18 && path == 'lib/ui/typography.dart') {
            h4Count++;
            continue;
          }
          if ((size == 26 || size == 34) &&
              const {
                'lib/ui/typography.dart',
                'lib/ui/sign_in_hero.dart',
                'lib/ui/sign_in_pane.dart',
              }.contains(path)) {
            continue;
          }
          unexpected.add('$path: fontSize $size');
        }
        if (path != 'lib/editor/marks.dart' &&
            RegExp(r'FontWeight\.(?:w[123789]00|bold)\b').hasMatch(code)) {
          unexpected.add('$path: unreviewed weight');
        }
        for (final match in RegExp(
          r'\bletterSpacing:\s*(-?\d+(?:\.\d+)?)',
        ).allMatches(code)) {
          if (double.parse(match[1]!) != 0) {
            unexpected.add('$path: letter tracking');
          }
        }
      }
      expect(unexpected, isEmpty);
      expect(
        h4Count,
        1,
        reason:
            '18px is the single editor H4 definition, verified by document_typography_test.',
      );
    },
  );
}
