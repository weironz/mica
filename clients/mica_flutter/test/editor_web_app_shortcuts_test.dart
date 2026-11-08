// On the web a Ctrl/Cmd chord arrives with a `character` ("f" for Ctrl+F),
// and the editor's "text-producing key" branch answered it with
// skipRemainingHandlers — which on web stops the key before the app's
// shortcuts. Ctrl+F (in-page find, gh#167), Ctrl+Shift+F, Ctrl+N, Ctrl+, all
// did nothing while typing, and the browser opened its own find bar instead,
// which cannot step through text drawn on a canvas. Browser-only: the bug
// lives behind kIsWeb. Run via scripts/browser-tests.sh.
@TestOn('browser')
library;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  testWidgets('Ctrl+F in the editor reaches the app shortcut on web', (
    tester,
  ) async {
    var findShortcut = 0, plainF = 0;
    final writes = <DocOp>[];
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: CallbackShortcuts(
          bindings: {
            const SingleActivator(LogicalKeyboardKey.keyF, control: true): () =>
                findShortcut++,
            const SingleActivator(LogicalKeyboardKey.keyF): () => plainF++,
          },
          child: Scaffold(
            body: MicaEditor(
              rootBlockId: 'root',
              nodes: [EditorNode(id: 'a', kind: 'paragraph', text: 'hello')],
              version: 0,
              canEdit: true,
              onApplyOperations: (ops) async => writes.addAll(ops),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.tapAt(
      tester.getTopLeft(find.byType(DocumentSurface)) + const Offset(40, 12),
    );
    await tester.pump();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyF, character: 'f');
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    expect(findShortcut, 1, reason: 'Ctrl+F must bubble to the app shortcut');

    // A plain letter is still text: it stays with the editor's text input.
    await tester.sendKeyEvent(LogicalKeyboardKey.keyF, character: 'f');
    await tester.pump();
    expect(plainF, 0, reason: 'typing f must not trigger app shortcuts');

    await tester.pumpWidget(const SizedBox());
  });
}
