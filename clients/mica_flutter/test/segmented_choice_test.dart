import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/segmented_choice.dart';

void main() {
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
