import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/margin_selection.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

void main() {
  testWidgets('dragging in a side margin selects the full body range', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1200, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    final frameKey = GlobalKey();
    final bodyKey = GlobalKey();
    final selectionHook = EditorMarginSelectionHook();
    const lastText = 'third paragraph';
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: EditorMarginSelectionRegion(
            enabled: true,
            editorSurfaceKey: bodyKey,
            pageFrameKey: frameKey,
            hook: selectionHook,
            onOutsidePointerDown: () {},
            child: SingleChildScrollView(
              child: Center(
                child: SizedBox(
                  key: frameKey,
                  width: 520,
                  child: Padding(
                    key: bodyKey,
                    padding: const EdgeInsets.only(top: 4),
                    child: MicaEditor(
                      rootBlockId: 'root',
                      nodes: [
                        EditorNode(
                          id: 'first',
                          kind: 'paragraph',
                          text: 'first paragraph',
                        ),
                        EditorNode(
                          id: 'second',
                          kind: 'paragraph',
                          text: 'second paragraph',
                        ),
                        EditorNode(
                          id: 'third',
                          kind: 'paragraph',
                          text: lastText,
                        ),
                      ],
                      version: 0,
                      canEdit: true,
                      marginSelectionHook: selectionHook,
                      onApplyOperations: (_) async {},
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();

    final frame = tester.getRect(find.byKey(frameKey));
    final body = tester.getRect(find.byKey(bodyKey));
    final start = Offset(frame.left - 80, body.top + 2);
    final end = Offset(frame.right + 80, body.bottom - 2);
    final drag = await tester.startGesture(
      start,
      kind: PointerDeviceKind.mouse,
    );
    await drag.moveTo(end);
    await tester.pump();
    await drag.up();
    await tester.pump();

    final selection = tester
        .widget<DocumentSurface>(find.byType(DocumentSurface))
        .selection;
    expect(selection, isNotNull);
    expect(selection!.start, const DocPosition(0, 0));
    expect(selection.end, const DocPosition(2, lastText.length));

    final reverse = await tester.startGesture(
      Offset(frame.right + 80, body.bottom - 2),
      kind: PointerDeviceKind.mouse,
    );
    await reverse.moveTo(Offset(frame.left - 80, body.top + 2));
    await tester.pump();
    await reverse.up();
    await tester.pump();

    final reverseSelection = tester
        .widget<DocumentSurface>(find.byType(DocumentSurface))
        .selection!;
    expect(reverseSelection.anchor, const DocPosition(2, lastText.length));
    expect(reverseSelection.focus, const DocPosition(0, 0));
  });
}
