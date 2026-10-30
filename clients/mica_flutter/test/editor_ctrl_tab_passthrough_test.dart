import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// Ctrl+Tab switches page tabs (ui/tab_shortcuts.dart), which only works if the
// editor lets it bubble. Its Tab handler used to take every Tab, modifiers or
// not: Ctrl+Tab in a list item indented the item and never reached the app.
void main() {
  testWidgets('Ctrl+Tab in a list item bubbles; plain Tab still indents', (
    tester,
  ) async {
    final writes = <DocOp>[];
    var reachedApp = 0;
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: CallbackShortcuts(
          bindings: {
            const SingleActivator(LogicalKeyboardKey.tab, control: true): () =>
                reachedApp++,
          },
          child: Scaffold(
            body: SingleChildScrollView(
              child: MicaEditor(
                rootBlockId: 'root',
                nodes: [
                  EditorNode(id: 'a', kind: 'bulleted_list', text: 'one'),
                  EditorNode(id: 'b', kind: 'bulleted_list', text: 'two'),
                ],
                version: 0,
                canEdit: true,
                onApplyOperations: (ops) async => writes.addAll(ops),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    final second = tester.getTopLeft(find.byType(DocumentSurface));
    await tester.tapAt(second + const Offset(60, 40));
    await tester.pump();

    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    expect(reachedApp, 1, reason: 'Ctrl+Tab must reach the app shortcut');
    expect(writes, isEmpty, reason: 'Ctrl+Tab must not indent');

    await tester.sendKeyEvent(LogicalKeyboardKey.tab);
    await tester.pump();
    expect(reachedApp, 1);
    expect(writes, isNotEmpty, reason: 'plain Tab still indents the item');

    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  });
}
