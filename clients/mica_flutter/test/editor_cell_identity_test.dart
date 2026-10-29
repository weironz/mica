import 'package:flutter/material.dart' hide TableCell;
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'support/markdown_codec.dart';

// A stored table whose body cell carries inline-Markdown source (the legacy
// shape), decoded through the engine — what a real document hands the editor.
EditorNode table(String id, String value) => EditorNode(
  id: id,
  kind: 'table',
  text: '',
  data: TableData.fromBlock({
    'rows': [
      ['h'],
      [value],
    ],
  }).toBlockData(),
);
Future<void> pump(WidgetTester t, List<EditorNode> nodes, List<DocOp> ops, int version) async {
  await t.pumpWidget(MaterialApp(localizationsDelegates: AppLocalizations.localizationsDelegates,
    supportedLocales: AppLocalizations.supportedLocales, home: Scaffold(body: MicaEditor(
      key: const ValueKey('cell'), rootBlockId: 'root', nodes: nodes, version: version,
      canEdit: true, onApplyOperations: (batch) async => ops.addAll(batch)))));
  await t.pump();
}
Future<void> open(WidgetTester t) async {
  final r = t.renderObject<RenderDocument>(find.byType(DocumentSurface));
  await t.tapAt(t.getTopLeft(find.byType(DocumentSurface)) + r.tableCellRect(0, 1, 0)!.center);
  await t.pump(); await t.pump(const Duration(milliseconds: 50));
}
void main() {
  setUpAll(useNativeMarkdownCodec);
  testWidgets('cell draft follows table id across reorder', (t) async {
    final ops = <DocOp>[];
    await pump(t, [table('A', 'oldA'), table('B', 'oldB')], ops, 0);
    await open(t);
    await t.enterText(find.byType(TextField), 'draft'); await t.pump();
    await pump(t, [table('B', 'oldB'), table('A', 'oldA')], ops, 1);
    await t.sendKeyEvent(LogicalKeyboardKey.escape); await t.pump();
    expect(ops.single['block_id'], 'A');
    final nodes = t.widget<DocumentSurface>(find.byType(DocumentSurface)).nodes;
    expect(TableData.fromBlock(nodes[0].data).rows[1][0].text, 'oldB');
    expect(TableData.fromBlock(nodes[1].data).rows[1][0].text, 'draft');
    await t.pumpWidget(const SizedBox()); await t.pump();
  });
  testWidgets('untouched cell rebases remote value and closes without a write', (t) async {
    final ops = <DocOp>[];
    await pump(t, [table('A', 'oldA')], ops, 0); await open(t);
    await pump(t, [table('A', 'remoteA')], ops, 1);
    expect(t.widget<TextField>(find.byType(TextField)).controller!.text, 'remoteA');
    await t.sendKeyEvent(LogicalKeyboardKey.escape); await t.pump();
    expect(ops, isEmpty);
    expect(TableData.fromBlock(t.widget<DocumentSurface>(find.byType(DocumentSurface)).nodes.single.data).rows[1][0].text, 'remoteA');
    await t.pumpWidget(const SizedBox()); await t.pump();
  });
  testWidgets('remote cell rebasing preserves marks after reconciliation', (t) async {
    final ops = <DocOp>[];
    await pump(t, [table('A', 'one **two**')], ops, 0); await open(t);
    await pump(t, [table('A', 'one **three** longer')], ops, 1);
    await t.pump();
    final ctl = t.widget<TextField>(find.byType(TextField)).controller! as dynamic;
    expect(ctl.text, 'one three longer');
    expect(ctl.marks.single.type, 'bold');
    expect(ctl.marks.single.start, 4);
    expect(ctl.marks.single.end, 9);
    await t.sendKeyEvent(LogicalKeyboardKey.escape); await t.pump();
    expect(ops, isEmpty);
    final cell = TableData.fromBlock(
      t.widget<DocumentSurface>(find.byType(DocumentSurface)).nodes.single.data,
    ).rows[1][0];
    expect(cell.text, 'one three longer');
    expect(cell.marks.single.type, 'bold');
    await t.pumpWidget(const SizedBox()); await t.pump();
  });
  testWidgets('removed table closes a dirty cell without writing another block', (t) async {
    final ops = <DocOp>[];
    await pump(t, [table('A', 'oldA'), table('B', 'oldB')], ops, 0); await open(t);
    await t.enterText(find.byType(TextField), 'draft'); await t.pump();
    await pump(t, [table('B', 'oldB')], ops, 1); await t.pump();
    expect(find.byType(TextField), findsNothing);
    expect(ops, isEmpty);
    await t.pumpWidget(const SizedBox()); await t.pump();
  });
}
