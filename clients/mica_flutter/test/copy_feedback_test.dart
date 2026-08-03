// Copying used to be announced by a SnackBar — a black bar across the bottom
// of the window reporting that the expected thing happened, nowhere near the
// button that was pressed. Every copy control that stays on screen now answers
// in place: its icon becomes a green check for a moment.
//
// The two here are the ones with no widget to hang a test off: the code
// block's copy button is PAINTED by the render object (so the confirmation is
// paint state threaded through `copiedCode`), and the link hover bar used to
// close itself on copy and say nothing at all.
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  Future<void> pumpEditor(WidgetTester tester, List<EditorNode> nodes) async {
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (batch) async {},
          ),
        ),
      ),
    );
    await tester.pump();
  }

  RenderDocument renderOf(WidgetTester tester) =>
      tester.renderObject<RenderDocument>(find.byType(DocumentSurface));

  group('the code block copy button', () {
    /// Hover the block so the toolbar lays out, then click the copy button
    /// where the renderer says it is.
    Future<RenderDocument> copyFirstCodeBlock(WidgetTester tester) async {
      await pumpEditor(tester, [
        EditorNode(id: 'c', kind: 'code_block', text: 'print("hi")\n'),
      ]);
      final origin = tester.getTopLeft(find.byType(MicaEditor));
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: origin + const Offset(60, 20));
      await tester.pump();

      final r = renderOf(tester);
      final button = r.debugCopyButtonAt(0);
      expect(
        button,
        isNotNull,
        reason: 'hovering the block must lay the toolbar out',
      );
      await tester.tapAt(origin + button!.center);
      await tester.pump();
      return r;
    }

    testWidgets('confirms on the button, not in a bar at the bottom', (
      tester,
    ) async {
      final r = await copyFirstCodeBlock(tester);

      expect(r.debugCopiedCode, 0);
      // The regression this replaces: a SnackBar for a copy that worked.
      expect(find.byType(SnackBar), findsNothing);
    });

    /// The toolbar is painted only while the block is hovered, and the click
    /// itself can drop the hover — first cut left the check on a strip that
    /// had already stopped being drawn, so nothing at all was visible.
    testWidgets('the strip stays up even after the hover is lost', (
      tester,
    ) async {
      final r = await copyFirstCodeBlock(tester);
      r.setHover(null);
      await tester.pump();

      expect(r.debugCopiedCode, 0);
      expect(r.debugCodeToolbarVisible(0), isTrue);

      await tester.pump(const Duration(seconds: 2));
      expect(r.debugCodeToolbarVisible(0), isFalse);
    });

    /// A button stuck on "done" stops being a button you can press again — the
    /// next copy would look like nothing happened.
    testWidgets('the check clears itself', (tester) async {
      final r = await copyFirstCodeBlock(tester);
      expect(r.debugCopiedCode, 0);

      await tester.pump(const Duration(seconds: 2));
      expect(r.debugCopiedCode, isNull);
    });
  });

  group('the link hover bar', () {
    /// Hovering a link opens the bar; clicking 复制 used to close it instantly
    /// and give no sign the copy had happened at all.
    testWidgets('stays open and says 已复制', (tester) async {
      // The check only goes up once the clipboard write REPORTS success, so
      // the channel has to answer — unstubbed it never completes under the
      // test clock and the button would sit on its idle face forever.
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async => null,
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );

      await pumpEditor(tester, [
        EditorNode(
          id: 'p',
          kind: 'paragraph',
          text: 'see the docs',
          data: {
            'marks': marksToJson([
              Mark(4, 12, 'link', href: 'https://mica.dev'),
            ]),
          },
        ),
      ]);
      // Aim at the middle of the linked run rather than a guessed pixel: the
      // renderer knows where character 8 of `see the docs` landed.
      final r = renderOf(tester);
      final caret = r.caretRectFor(const DocPosition(0, 8));
      expect(caret, isNotNull);
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      await mouse.moveTo(r.localToGlobal(caret!.center));
      await tester.pump(const Duration(milliseconds: 50));

      expect(
        find.text('复制链接'),
        findsOneWidget,
        reason: 'the bar should open',
      );
      await tester.tap(find.text('复制链接'));
      await tester.pump();
      await tester.pump();

      expect(find.text('已复制'), findsOneWidget);
      expect(find.byType(SnackBar), findsNothing);
    });
  });
}
