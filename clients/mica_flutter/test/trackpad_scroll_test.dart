// A two-finger swipe on a laptop touchpad must scroll the page, not select.
//
// Reported 2026-10-21 (Windows): with the caret in the body, swiping up/down on
// the touchpad drew a text selection instead of moving the page. Precision
// touchpads reach Flutter as PointerPanZoom events of kind `trackpad`, and a
// drag recognizer accepts those as a drag by default — so the editor's
// selection pan competed with the host's scroll view, and won whenever one
// report crossed both slops (see the swipe below).
//
// The editor does not own the scrollable (the host does), so this test owns
// one, and "did it scroll / did the caret move" are numbers.
import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  testWidgets(
    'touchpad two-finger swipe scrolls the page and keeps the caret',
    (tester) async {
      final scroll = ScrollController();
      String? cursor;
      await tester.pumpWidget(
        MaterialApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: Scaffold(
            body: SizedBox(
              height: 240,
              child: SingleChildScrollView(
                controller: scroll,
                child: MicaEditor(
                  rootBlockId: 'root',
                  nodes: [
                    for (var i = 0; i < 40; i++)
                      EditorNode(id: 'p$i', kind: 'paragraph', text: 'line $i'),
                  ],
                  version: 0,
                  canEdit: true,
                  onApplyOperations: (_) async {},
                  onSelectionChanged: (block, offset) =>
                      cursor = '$block:$offset',
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pump();

      // Caret in the body first — the reported precondition.
      final at =
          tester.getTopLeft(find.byType(MicaEditor)) + const Offset(60, 14);
      await tester.tapAt(at);
      await tester.pump();
      final placed = cursor;
      expect(placed, isNotNull);

      // Fingers moving UP = content moves up = scroll down.
      final swipe = await tester.createGesture(
        kind: PointerDeviceKind.trackpad,
      );
      await swipe.panZoomStart(at);
      // A brisk swipe: the first report already moves past BOTH recognizers'
      // slop (18 px for the scroll view's vertical drag, 36 px for the editor's
      // pan). In that one event the deeper editor is asked first and wins — the
      // slow, 12-px-a-frame swipe never reproduced the bug, because the scroll
      // view crossed its smaller slop a frame earlier.
      for (var i = 1; i <= 5; i++) {
        await swipe.panZoomUpdate(at, pan: Offset(0, -48.0 * i));
        await tester.pump();
      }
      await swipe.panZoomEnd();
      await tester.pumpAndSettle();

      expect(scroll.offset, greaterThan(0), reason: 'the page did not scroll');
      expect(
        cursor,
        placed,
        reason: 'the swipe moved the caret / selected text',
      );
    },
    variant: TargetPlatformVariant.desktop(),
  );
}
