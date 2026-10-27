import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  testWidgets('CF_HTML paste imports tagged multibyte fragment without context', (t) async {
    const pre = '<html><body>前文 before <!--StartFragment-->';
    const fragment = '<p><strong>選択 &amp; text</strong></p>\n<p>next</p>';
    const post = '<!--EndFragment--> after 後文</body></html>';
    const header = 'Version:1.0\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n';
    final start = utf8.encode(header).length;
    String pad(int n) => n.toString().padLeft(10, '0');
    final html = 'Version:1.0\r\nStartHTML:${pad(start)}\r\nEndHTML:${pad(start + utf8.encode('$pre$fragment$post').length)}\r\nStartFragment:${pad(start + utf8.encode(pre).length)}\r\nEndFragment:${pad(start + utf8.encode('$pre$fragment').length)}\r\n$pre$fragment$post';
    final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(const MethodChannel('pasteboard'), (call) async => call.method == 'html' ? html : null);
    messenger.setMockMethodCallHandler(SystemChannels.platform, (call) async => call.method == 'Clipboard.getData' ? {'text': '選択 & text\nnext'} : null);
    addTearDown(() {
      messenger.setMockMethodCallHandler(const MethodChannel('pasteboard'), null);
      messenger.setMockMethodCallHandler(SystemChannels.platform, null);
    });
    await t.pumpWidget(MaterialApp(localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales, home: Scaffold(body: MicaEditor(rootBlockId: 'root',
        nodes: [EditorNode(id: 'a', kind: 'paragraph', text: '')], version: 0, canEdit: true, onApplyOperations: (_) async {}))));
    await t.pump();
    final render = t.renderObject<RenderDocument>(find.byType(DocumentSurface));
    await t.tapAt(t.getTopLeft(find.byType(DocumentSurface)) + render.caretRectFor(const DocPosition(0, 0))!.center);
    await t.pump();
    await t.sendKeyDownEvent(LogicalKeyboardKey.controlLeft); await t.sendKeyEvent(LogicalKeyboardKey.keyV); await t.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await t.pump(const Duration(milliseconds: 500));
    final nodes = t.widget<DocumentSurface>(find.byType(DocumentSurface)).nodes;
    expect(nodes.map((n) => n.text).join('\n'), '選択 & text\nnext');
    expect(nodes.map((n) => n.text).join(), isNot(contains('before')));
    expect(marksFromData(nodes.first.data).any((m) => m.type == 'bold'), isTrue);
    await t.pumpWidget(const SizedBox()); await t.pump();
  },
      // `Pasteboard.html` returns null off Windows (pasteboard 0.5.0
      // pasteboard_platform_io.dart), so elsewhere this paste can only take
      // the plain-text path. Runs on Windows in flutter-integration.yml.
      skip: !Platform.isWindows);
}
