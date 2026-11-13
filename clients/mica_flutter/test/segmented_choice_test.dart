import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/segmented_choice.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  for (final dark in [false, true]) {
    for (final labels in [
      ['跟随系统', '浅色', '深色'],
      ['Follow system', 'Light', 'Dark'],
    ]) {
      testWidgets('group fits its options in stretched rows: $dark $labels', (
        t,
      ) async {
        for (final width in [640.0, 180.0]) {
          await t.pumpWidget(
            MaterialApp(
              theme: (dark ? MicaTokens.dark_ : MicaTokens.light)
                  .toMaterialTheme(),
              home: Scaffold(
                body: Align(
                  alignment: Alignment.topLeft,
                  child: SizedBox(
                    width: width,
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        MicaSegmentGroup(
                          children: [
                            for (final label in labels)
                              MicaSegmentButton(
                                label: Text(label),
                                selected: label == labels.first,
                                onSelected: (_) {},
                              ),
                          ],
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          );
          final group = find.byType(MicaSegmentGroup);
          final background = find
              .descendant(of: group, matching: find.byType(DecoratedBox))
              .first;
          final bounds = t.getRect(background);
          final options = [
            for (final label in labels)
              t.getRect(
                find.ancestor(
                  of: find.text(label),
                  matching: find.byType(MicaSegmentButton),
                ),
              ),
          ];
          expect(bounds.left, t.getRect(group).left);
          expect(bounds.width, lessThanOrEqualTo(width));
          expect(options.first.left, bounds.left + 2);
          expect(
            bounds.right,
            options.map((r) => r.right).reduce((a, b) => a > b ? a : b) + 2,
          );
          if (width == 640) {
            expect(options.last.top, options.first.top);
            expect(bounds.width, lessThan(width));
          } else {
            expect(options.last.top, greaterThan(options.first.top));
          }
          expect(t.takeException(), isNull);
        }
      });
    }
  }
  testWidgets('selection and disabled options preserve the existing actions', (
    t,
  ) async {
    var chosen = false;
    var calls = 0;
    await t.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: StatefulBuilder(
            builder: (context, setState) => MicaSegmentGroup(
              children: [
                MicaSegmentButton(
                  label: const Text('Choice'),
                  selected: chosen,
                  onSelected: (value) {
                    calls++;
                    setState(() => chosen = value);
                  },
                ),
                const MicaSegmentButton(
                  label: Text('Disabled'),
                  selected: false,
                  onSelected: null,
                ),
              ],
            ),
          ),
        ),
      ),
    );
    await t.tap(find.text('Choice'));
    await t.pumpAndSettle();
    expect(chosen, isTrue);
    expect(calls, 1);
    await t.tap(find.text('Disabled'));
    await t.pumpAndSettle();
    expect(calls, 1);
  });
  test('all old ChoiceChip consumers migrated', () {
    final found = Directory('lib')
        .listSync(recursive: true)
        .whereType<File>()
        .where(
          (f) =>
              f.path.endsWith('.dart') &&
              RegExp(r'\bChoiceChip\s*\(').hasMatch(f.readAsStringSync()),
        );
    expect(found, isEmpty);
  });
}
