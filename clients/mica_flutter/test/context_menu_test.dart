import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// Body-text right-click context menu: 复制/剪切 only over a ranged selection,
// 粘贴/粘贴为纯文本 whenever editable — and paste-as-plain inserts the text
// LITERALLY (no Markdown parsing), observed through the op stream.
void main() {
  Future<List<Map<String, dynamic>>> pumpEditor(
    WidgetTester tester,
    List<EditorNode> nodes,
  ) async {
    final ops = <Map<String, dynamic>>[];
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
            onApplyOperations: (batch) async => ops.addAll(batch),
          ),
        ),
      ),
    );
    await tester.pump();
    return ops;
  }

  Future<void> rightClickAt(WidgetTester tester, Offset globalPos) async {
    final g = await tester.startGesture(
      globalPos,
      kind: PointerDeviceKind.mouse,
      buttons: kSecondaryButton,
    );
    await g.up();
    await tester.pumpAndSettle();
  }

  testWidgets('right-click with no selection offers paste but not copy',
      (tester) async {
    await pumpEditor(tester, [
      EditorNode(id: 'p', kind: 'paragraph', text: 'hello world here'),
    ]);
    final origin = tester.getTopLeft(find.byType(MicaEditor));
    await rightClickAt(tester, origin + const Offset(80, 14));

    expect(find.text('粘贴'), findsOneWidget);
    expect(find.text('粘贴为纯文本'), findsOneWidget);
    expect(find.text('复制'), findsNothing);
    expect(find.text('剪切'), findsNothing);
  });

  testWidgets('right-click inside a drag selection offers copy + cut',
      (tester) async {
    await pumpEditor(tester, [
      EditorNode(id: 'p', kind: 'paragraph', text: 'hello world here'),
    ]);
    final origin = tester.getTopLeft(find.byType(MicaEditor));

    // Double-click a word (the editor's own multi-tap word select — two taps
    // within 400ms/12px), then right-click the same spot: inside the selection.
    final wordPos = origin + const Offset(100, 14);
    await tester.tapAt(wordPos);
    await tester.pump(const Duration(milliseconds: 60));
    await tester.tapAt(wordPos);
    await tester.pump();

    await rightClickAt(tester, wordPos);

    expect(find.text('复制'), findsOneWidget);
    expect(find.text('剪切'), findsOneWidget);
    expect(find.text('粘贴'), findsOneWidget);
    expect(find.text('粘贴为纯文本'), findsOneWidget);
  });

  testWidgets('粘贴为纯文本 inserts clipboard text literally — no Markdown parse',
      (tester) async {
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method == 'Clipboard.getData') {
          return <String, dynamic>{'text': '# not a heading\n**not bold**'};
        }
        return null;
      },
    );
    addTearDown(() => tester.binding.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, null));

    final ops = await pumpEditor(tester, [
      EditorNode(id: 'p', kind: 'paragraph', text: 'hello world here'),
    ]);
    final origin = tester.getTopLeft(find.byType(MicaEditor));
    await rightClickAt(tester, origin + const Offset(80, 14));

    await tester.tap(find.text('粘贴为纯文本'));
    await tester.pumpAndSettle();

    // Both lines arrive as literal PARAGRAPHS: `#` did not become a heading,
    // `**` stayed in the text instead of becoming a bold mark.
    final blocks = [
      for (final o in ops)
        if (o['type'] == 'insert_block') o['block'] as Map<String, dynamic>,
    ];
    expect(blocks, hasLength(2));
    expect(blocks.every((b) => b['type'] == 'paragraph'), isTrue);
    expect(blocks.map((b) => b['text']),
        containsAll(['# not a heading', '**not bold**']));
  });
}
