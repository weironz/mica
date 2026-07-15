import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

// Settings is its own route: built once, never rebuilt by the parent. So the
// Server page's list and active connection have to be READ through a getter,
// not captured at showDialog time — a snapshot went stale the moment a server
// was added or removed, and the symptom was brutal: the add landed in prefs
// while the dropdown denied it, and adding again then said "already in the
// list". Deleting looked equally broken.

void main() {
  testWidgets('a captured list goes stale; a getter does not', (tester) async {
    // The shell's truth, mutated WITHOUT rebuilding the "dialog" — which is
    // exactly what being a separate route means.
    var servers = <String>['local', 'https://a.example.com'];

    late StateSetter setDialog;
    List<String>? sawSnapshot;
    List<String>? sawGetter;

    final snapshot = servers; // captured at "showDialog" time
    List<String> live() => servers;

    await tester.pumpWidget(
      MaterialApp(
        home: StatefulBuilder(
          builder: (context, setState) {
            setDialog = setState;
            sawSnapshot = snapshot;
            sawGetter = live();
            return const SizedBox();
          },
        ),
      ),
    );

    servers = [...servers, 'https://b.example.com']; // the add happens
    setDialog(() {}); // the dialog rebuilds itself — what our setState does
    await tester.pump();

    expect(sawSnapshot, hasLength(2),
        reason: 'the captured list cannot see the add — this WAS the bug');
    expect(sawGetter, hasLength(3),
        reason: 'the getter reads the shell on every build');
    expect(sawGetter, contains('https://b.example.com'));
  });

  testWidgets('the dropdown follows a connection changed from outside',
      (tester) async {
    // Switching, or deleting the live server (which drops us to 本地模式),
    // changes the selection without anyone touching the dropdown. It has to
    // move.
    //
    // This also pins a Flutter behaviour worth stating: DropdownButtonFormField
    // DOES adopt a changed `initialValue` — _DropdownButtonFormFieldState
    // overrides didUpdateWidget to setValue(). FormFieldState, its base, does
    // not, and reading only the base is how this was briefly "fixed" by
    // hand-rolling a DropdownButton instead. If this test ever fails, Flutter
    // changed and the comment in _serverSection needs revisiting.
    const items = ['local', 'https://a.example.com'];
    Widget host(String value) => MaterialApp(
          home: Scaffold(
            body: DropdownButtonFormField<String>(
              initialValue: value,
              items: [
                for (final v in items)
                  DropdownMenuItem(value: v, child: Text(v)),
              ],
              onChanged: (_) {},
            ),
          ),
        );

    await tester.pumpWidget(host('local'));
    expect(find.text('local'), findsWidgets);

    await tester.pumpWidget(host('https://a.example.com'));
    await tester.pump();
    expect(find.text('https://a.example.com'), findsWidgets);
    expect(find.text('local'), findsNothing,
        reason: 'it must show where we are now, not where we started');
  });
}
