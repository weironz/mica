import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/ui/dialog_controllers.dart';

/// The bug this exists for killed the app, twice, in the two dialogs a new user
/// meets first: signing in and adding a server. The controller was disposed when
/// `showDialog`'s future resolved, but the route's exit transition still had the
/// TextField mounted — and a rebuild in that window ("A TextEditingController was
/// used after being disposed" → red screen) was not a race but the normal path,
/// because both callers setState right after the pop.
void main() {
  testWidgets('the controller survives a rebuild during the exit transition', (
    tester,
  ) async {
    String? popped;
    var extra = 0;

    await tester.pumpWidget(
      MaterialApp(
        home: StatefulBuilder(
          builder: (context, setState) => Scaffold(
            body: Column(
              children: [
                // Stands in for the app tree that rebuilds after the dialog pops.
                Text('rebuilds:$extra'),
                TextButton(
                  onPressed: () async {
                    popped = await showDialog<String>(
                      context: context,
                      builder: (context) => DialogTextControllers(
                        count: 1,
                        builder: (context, fields) => AlertDialog(
                          content: TextField(controller: fields[0]),
                          actions: [
                            TextButton(
                              onPressed: () =>
                                  Navigator.pop(context, fields[0].text),
                              child: const Text('ok'),
                            ),
                          ],
                        ),
                      ),
                    );
                    // The caller's setState — the thing that used to rebuild the
                    // leaving route and touch the disposed controller.
                    setState(() => extra++);
                  },
                  child: const Text('open'),
                ),
              ],
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.text('open'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'https://mica.example.com');
    await tester.tap(find.text('ok'));

    // Mid-transition: one frame, not settled. This is the window that crashed.
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
    expect(popped, 'https://mica.example.com');
    expect(find.text('rebuilds:1'), findsOneWidget);
  });

  test('the widget takes as many fields as the dialog has', () {
    // A three-field form (email / name / password) is the case that had three
    // separate disposals to get wrong.
    const w = DialogTextControllers(count: 3, builder: _noopBuilder);
    expect(w.count, 3);
  });

  testWidgets('initial text is seeded per field, extras start empty', (
    tester,
  ) async {
    // Rename-a-workspace opens with the current name already in the box; losing
    // that would turn a rename into a retype.
    late List<TextEditingController> seen;
    await tester.pumpWidget(
      MaterialApp(
        home: DialogTextControllers(
          count: 3,
          initialTexts: const ['first'],
          builder: (context, fields) {
            seen = fields;
            return const SizedBox();
          },
        ),
      ),
    );

    expect(seen.map((f) => f.text).toList(), ['first', '', '']);
  });
}

Widget _noopBuilder(BuildContext context, List<TextEditingController> fields) =>
    const SizedBox();
