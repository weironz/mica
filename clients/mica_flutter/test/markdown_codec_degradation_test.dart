import 'package:flutter/material.dart' hide TableCell;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';

import 'support/markdown_codec.dart';

// What the user gets when the Markdown engine is unavailable (Web without its
// wasm, desktop without the DLL) — 02 §5.2: paste falls back to plain text,
// copy writes text/plain only, legacy string table cells show their source.
// Each case runs the real editor path with the engine swapped out mid-test.
void main() {
  setUpAll(useNativeMarkdownCodec);
  void withoutEngine() {
    debugSetMarkdownCodec(const UnavailableMarkdownCodec());
    addTearDown(useNativeMarkdownCodec);
  }

  testWidgets('paste: multi-line Markdown lands as literal text', (t) async {
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    // No image / HTML flavor on the clipboard — only the plain text.
    messenger.setMockMethodCallHandler(
      const MethodChannel('pasteboard'),
      (_) async => null,
    );
    messenger.setMockMethodCallHandler(SystemChannels.platform, (call) async {
      return call.method == 'Clipboard.getData'
          ? {'text': '# Title\n**bold** line'}
          : null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(const MethodChannel('pasteboard'), null);
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
    withoutEngine();
    await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await t.sendKeyEvent(LogicalKeyboardKey.keyV);
    await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await t.pump(const Duration(milliseconds: 500));
    final nodes = t.widget<DocumentSurface>(find.byType(DocumentSurface)).nodes;
    expect(nodes.map((n) => n.text).join('\n'), contains('# Title'));
    expect(nodes.map((n) => n.text).join('\n'), contains('**bold** line'));
    expect(nodes.any((n) => n.kind == 'heading'), isFalse,
        reason: 'no engine, so nothing may be parsed as Markdown');
    await t.pumpWidget(const SizedBox());
    await t.pump();
  });

  test('copy: the selection keeps its plain text and drops the HTML flavor', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {})
      ..load([
        EditorNode(id: 'a', kind: 'heading', text: 'Title', data: {'level': 1}),
        EditorNode(id: 'b', kind: 'paragraph', text: 'body'),
      ]);
    addTearDown(c.dispose);
    c.selection = DocSelection(
      anchor: const DocPosition(0, 0),
      focus: const DocPosition(1, 4),
    );
    withoutEngine();
    expect(c.selectionHtml(), '',
        reason: '_copySelection writes text/plain only when HTML is empty');
    expect(c.selectionClipboardText(), contains('Title'));
    expect(c.selectionClipboardText(), contains('body'));
  });

  test('table: a legacy source-string cell shows its source verbatim', () {
    withoutEngine();
    final table = TableData.fromBlock({
      'rows': [
        ['**legacy** cell', 'plain'],
      ],
    });
    expect(table.rows[0][0], const TableCell('**legacy** cell'));
    expect(table.rows[0][1], const TableCell('plain'));
  });
}
