// Moving the caret below the fold must scroll it back into view.
//
// Measured broken 2026-08-05 with a real Chrome + CDP probe
// (`e2e/web_ime_probe.mjs`): after 28 typed lines the viewport was still pinned
// at the top of the document and the caret was nowhere on screen — you were
// typing into a place you could not see.
//
// Why it survived a whole milestone unnoticed is structural: the editor does
// NOT own the scrollable, the host does, so there was no ScrollController in
// the editor that anyone could notice was never being driven. The fix asks the
// enclosing viewport via `showOnScreen`; this test owns that viewport, so "did
// it scroll" is a number rather than a screenshot.
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  Future<ScrollController> pumpTallDoc(WidgetTester tester) async {
    final controller = ScrollController();
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: SizedBox(
            height: 240,
            child: SingleChildScrollView(
              controller: controller,
              child: MicaEditor(
                rootBlockId: 'root',
                nodes: [
                  for (var i = 0; i < 40; i++)
                    EditorNode(id: 'p$i', kind: 'paragraph', text: 'line $i'),
                ],
                version: 0,
                canEdit: true,
                onApplyOperations: (_) async {},
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    return controller;
  }

  testWidgets('moving the caret below the fold scrolls it into view', (
    tester,
  ) async {
    final scroll = await pumpTallDoc(tester);
    expect(scroll.offset, 0, reason: 'starts at the top');

    // Focus, then walk the caret far past the 240px viewport. Arrow keys go
    // through the same "we moved the caret" funnel typing does.
    await tester.tapAt(
      tester.getTopLeft(find.byType(MicaEditor)) + const Offset(60, 14),
    );
    await tester.pump();
    for (var i = 0; i < 30; i++) {
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.pump();
    }
    // The reveal is deferred one frame on purpose (layout must be current) and
    // it animates — so settle rather than pump once.
    await tester.pumpAndSettle();

    expect(
      scroll.offset,
      greaterThan(0),
      reason: 'the viewport never followed the caret — that is the bug itself',
    );
  });

  testWidgets('a caret already on screen leaves the view alone', (tester) async {
    final scroll = await pumpTallDoc(tester);
    await tester.tapAt(
      tester.getTopLeft(find.byType(MicaEditor)) + const Offset(60, 14),
    );
    await tester.pump();
    // One line down is still comfortably inside a 240px viewport.
    await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
    await tester.pumpAndSettle();

    // Scrolling on every keystroke would fight the reader: a caret that is
    // already visible must leave the viewport exactly where it is.
    expect(scroll.offset, 0);
  });
}
