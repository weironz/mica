import 'package:flutter/material.dart' hide TableCell;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/paste_repair.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';

import 'support/markdown_codec.dart';

// #185: one line of plain text pasted from a Markdown source landed with its
// backticks showing, because a single-line plain paste always inserted
// literally. It now weaves in with the marks the engine parsed — when they are
// ones prose does not produce by accident.
void main() {
  setUpAll(useNativeMarkdownCodec);

  group('singleLinePasteKeepsMarks', () {
    test('trusted: code, bold, strike, link, math', () {
      for (final t in ['code', 'bold', 'strike', 'link', 'math']) {
        expect(singleLinePasteKeepsMarks([t]), isTrue, reason: t);
      }
      expect(singleLinePasteKeepsMarks(['code', 'bold']), isTrue);
    });
    test('a lone italic, raw HTML or a footnote keeps the line literal', () {
      expect(singleLinePasteKeepsMarks(['italic']), isFalse);
      expect(singleLinePasteKeepsMarks(['code', 'italic']), isFalse);
      expect(singleLinePasteKeepsMarks(['html']), isFalse);
      expect(singleLinePasteKeepsMarks(['footnote']), isFalse);
    });
    test('nothing to apply is not a reason to parse', () {
      expect(singleLinePasteKeepsMarks(const []), isFalse);
    });
  });

  Future<List<EditorNode>> paste(WidgetTester t, String clip) async {
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    // Plain text only: no image or HTML flavor on the clipboard.
    messenger.setMockMethodCallHandler(
      const MethodChannel('pasteboard'),
      (_) async => null,
    );
    messenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      return call.method == 'Clipboard.getData' ? {'text': clip} : null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(
        const MethodChannel('pasteboard'),
        null,
      );
      messenger.setMockMethodCallHandler(SystemChannels.platform, null);
    });
    await t.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: [EditorNode(id: 'a', kind: 'paragraph', text: '')],
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
          ),
        ),
      ),
    );
    await t.pump();
    final render = t.renderObject<RenderDocument>(find.byType(DocumentSurface));
    await t.tapAt(
      t.getTopLeft(find.byType(DocumentSurface)) +
          render.caretRectFor(const DocPosition(0, 0))!.center,
    );
    await t.pump();
    await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.keyV);
    await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await t.pump(const Duration(milliseconds: 500));
    final nodes = t.widget<DocumentSurface>(find.byType(DocumentSurface)).nodes;
    await t.pumpWidget(const SizedBox());
    await t.pump();
    return nodes;
  }

  testWidgets('#185: a code span after CJK text pastes as inline code', (
    t,
  ) async {
    const clip = '已实测的单盘路径`PredictedMediaLifeLeftPercent` 的方向是剩余百分比。';
    final nodes = await paste(t, clip);
    expect(nodes, hasLength(1));
    final n = nodes.single;
    expect(n.text, '已实测的单盘路径PredictedMediaLifeLeftPercent 的方向是剩余百分比。');
    final code = marksFromData(n.data).where((m) => m.type == 'code').single;
    expect(
      n.text.substring(code.start, code.end),
      'PredictedMediaLifeLeftPercent',
    );
  });

  testWidgets('prose with stray asterisks stays literal', (t) async {
    final nodes = await paste(t, 'compute 2*3*4 here');
    expect(nodes.single.text, 'compute 2*3*4 here');
    expect(marksFromData(nodes.single.data), isEmpty);
  });

  testWidgets('no engine: the line pastes literally, no crash', (t) async {
    debugSetMarkdownCodec(const UnavailableMarkdownCodec());
    addTearDown(useNativeMarkdownCodec);
    final nodes = await paste(t, 'run `ls` now');
    expect(nodes.single.text, 'run `ls` now');
  });
}
