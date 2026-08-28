import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

// Reported 2026-08-28: a table sat in a bigger hole than the rest of the page,
// and a deeper one below than above. Both come from the same place — the box
// reserves two blank strips (column handles on top, the add-row bar under),
// they were 10 and 16, and the ordinary block gap was added on top of both.
// Neither strip paints anything until the pointer is over the table, so what
// the reader sees is just space.
//
// The assertions are on the GRID rect, not the box: the box includes the
// strips, so measuring it would measure around the very thing that was wrong.
void main() {
  EditorNode table(String id) => EditorNode(
    id: id,
    kind: 'table',
    text: '',
    data: {
      'columns': 2,
      'header': true,
      'rows': [
        ['项', '值'],
        ['地址', 'http://192.168.73.5:3000'],
      ],
    },
  );

  Future<RenderDocument> pump(WidgetTester tester, List<EditorNode> nodes) async {
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
            onApplyOperations: (_) async {},
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    return tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
  }

  testWidgets('the blank space above and below a table is equal', (
    tester,
  ) async {
    final r = await pump(tester, [
      EditorNode(id: 'a', kind: 'paragraph', text: '数据库采用 Postgres。'),
      table('t'),
      EditorNode(id: 'b', kind: 'paragraph', text: '用户名或邮箱都能登录。'),
    ]);

    final (aTop, aHeight) = r.debugBoxAt(0);
    final grid = r.debugTableGridAt(1)!;
    final (bTop, _) = r.debugBoxAt(2);

    final above = grid.top - (aTop + aHeight);
    final below = bTop - grid.bottom;
    expect(above, below, reason: 'the table must not hang low in its own box');
  });

  testWidgets('that space stays in the page rhythm', (tester) async {
    // "Compact" against the only yardstick that means anything: the gap
    // between two ordinary paragraphs. A table may not open a wider hole.
    final r = await pump(tester, [
      EditorNode(id: 'a', kind: 'paragraph', text: '一段'),
      EditorNode(id: 'b', kind: 'paragraph', text: '又一段'),
      table('t'),
      EditorNode(id: 'c', kind: 'paragraph', text: '再一段'),
    ]);

    final (aTop, aHeight) = r.debugBoxAt(0);
    final (bTop, bHeight) = r.debugBoxAt(1);
    final paragraphGap = bTop - (aTop + aHeight);
    final grid = r.debugTableGridAt(2)!;

    final above = grid.top - (bTop + bHeight);
    final (cTop, _) = r.debugBoxAt(3);
    final below = cTop - grid.bottom;

    expect(above, lessThanOrEqualTo(paragraphGap + 1));
    expect(below, lessThanOrEqualTo(paragraphGap + 1));
    expect(above, greaterThan(6), reason: 'compact, not glued to the text');
  });

  testWidgets('a heading after a table keeps its air', (tester) async {
    // The gap is charged against the table's strip, and a heading's 30px of
    // section air must not be charged away with it.
    final r = await pump(tester, [
      table('t'),
      EditorNode(id: 'h', kind: 'heading', text: '一、访问信息', data: {'level': 2}),
    ]);
    final grid = r.debugTableGridAt(0)!;
    final (hTop, _) = r.debugBoxAt(1);
    expect(hTop - grid.bottom, greaterThan(20));
  });
}
