// Canvas-following overlays (format bar, link bar, slash menu, cell editor)
// show their buttons' tooltips again.
//
// They had to be switched off (2026-08-12): the bars followed the canvas with
// a CompositedTransformFollower, and a Tooltip here is an OverlayPortal that
// asks its anchor for a paint transform DURING layout. A follower layer has
// none until compositing, so every hover asserted. The bars now follow with
// CanvasFollower (editor/canvas_follower.dart), which places them at paint
// time with no follower layer in the chain.
import 'package:flutter/gestures.dart' show PointerDeviceKind;
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  // `semanticsEnabled: false`: testWidgets turns semantics on by default, and
  // with semantics on the bars keep tooltips off (see the next test).
  testWidgets(
    'hovering a format-bar button shows its tooltip',
    semanticsEnabled: false,
    (tester) async {
      final ops = <DocOp>[];
      await tester.pumpWidget(
        MaterialApp(
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          locale: const Locale('zh'),
          home: Scaffold(
            body: SingleChildScrollView(
              child: Column(
                children: [
                  const SizedBox(height: 200),
                  MicaEditor(
                    rootBlockId: 'root',
                    nodes: [
                      EditorNode(
                        id: 'p',
                        kind: 'paragraph',
                        text: 'select some of this text please',
                        data: const {},
                      ),
                    ],
                    version: 0,
                    canEdit: true,
                    onApplyOperations: (o) async => ops.addAll(o),
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.pump();
      final canvas = tester.getTopLeft(find.byType(DocumentSurface));
      final from = canvas + const Offset(30, 14);
      await tester.tapAt(from);
      await tester.pump();
      final g = await tester.startGesture(from);
      for (var i = 1; i <= 6; i++) {
        await g.moveTo(from + Offset(25.0 * i, 0));
        await tester.pump();
      }
      await g.up();
      await tester.pump(const Duration(milliseconds: 50));

      final tips = find.byType(Tooltip);
      expect(tips, findsWidgets, reason: 'the format bar must be up');
      final message = tester.widget<Tooltip>(tips.first).message!;
      final mouse = TestPointer(1, PointerDeviceKind.mouse);
      await tester.sendEventToBinding(
        mouse.hover(tester.getCenter(tips.first)),
      );
      await tester.pump(const Duration(seconds: 2));
      await tester.pump();

      expect(tester.takeException(), isNull);
      expect(find.text(message), findsOneWidget, reason: 'tooltip shown');
      await tester.sendEventToBinding(mouse.removePointer());

      // The bar is painted away from its own layout box; taps must still land.
      await tester.tap(tips.first);
      await tester.pump(const Duration(milliseconds: 500));
      expect(ops, isNotEmpty, reason: 'tapping the button applies its mark');
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('with semantics on, the bar keeps its tooltips off', (
    tester,
  ) async {
    // On the web an opening tooltip's announcement takes DOM focus from the
    // editor's text input and closes the bar under the pointer (seen in
    // Playwright with the accessibility tree on). Screen-reader sessions keep
    // the pre-CanvasFollower behaviour: no tooltips in these bars.
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [
              EditorNode(
                id: 'p',
                kind: 'paragraph',
                text: 'select some of this text please',
                data: const {},
              ),
            ],
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
          ),
        ),
      ),
    );
    await tester.pump();
    final from =
        tester.getTopLeft(find.byType(DocumentSurface)) + const Offset(30, 14);
    await tester.tapAt(from);
    await tester.pump();
    final g = await tester.startGesture(from);
    for (var i = 1; i <= 6; i++) {
      await g.moveTo(from + Offset(25.0 * i, 0));
      await tester.pump();
    }
    await g.up();
    await tester.pump(const Duration(milliseconds: 50));
    expect(find.byType(IconButton), findsWidgets, reason: 'the bar is up');
    expect(find.byType(RawTooltip), findsNothing);
  });
}
