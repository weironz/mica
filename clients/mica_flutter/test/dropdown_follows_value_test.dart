import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

// Pins one Flutter behaviour the Settings dialog leans on: a
// DropdownButtonFormField DOES adopt a changed `initialValue`, because
// _DropdownButtonFormFieldState overrides didUpdateWidget to setValue().
// FormFieldState, its base, does not — and reading only the base is how a
// working dropdown once got hand-rolled into a DropdownButton for no reason.
//
// Who depends on it: the AI provider picker (dialogs.dart `_aiSection`) is
// built with `initialValue: _preset`, and `_load()` sets `_preset` from the
// server AFTER the first build. If Flutter ever changes, that dropdown silently
// shows the wrong provider while the fields below it show the right one.

void main() {
  testWidgets('a dropdown adopts an initialValue changed after first build', (
    tester,
  ) async {
    const items = ['openai', 'deepseek'];
    Widget host(String value) => MaterialApp(
      home: Scaffold(
        body: DropdownButtonFormField<String>(
          initialValue: value,
          items: [
            for (final v in items) DropdownMenuItem(value: v, child: Text(v)),
          ],
          onChanged: (_) {},
        ),
      ),
    );

    await tester.pumpWidget(host('openai'));
    expect(find.text('openai'), findsWidgets);

    // What _load()'s setState does: same widget, new value, nobody touched the
    // dropdown itself.
    await tester.pumpWidget(host('deepseek'));
    await tester.pump();
    expect(find.text('deepseek'), findsWidgets);
    expect(
      find.text('openai'),
      findsNothing,
      reason: 'it must show the value it was given, not the one it started on',
    );
  });
}
