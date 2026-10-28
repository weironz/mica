import 'dart:ui' as ui;

import 'package:flutter/material.dart' hide TableCell;
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

Future<void> _selectAll(WidgetTester tester) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.keyA);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await tester.pumpAndSettle();
}

const _canvasKey = ValueKey('editor-pixels');
typedef _Pixels = ({ByteData data, int width, int height});

Future<_Pixels> _capture(WidgetTester tester) async {
  final boundary = tester.renderObject<RenderRepaintBoundary>(
    find.byKey(_canvasKey),
  );
  return (await tester.runAsync(() async {
    final image = await boundary.toImage();
    try {
      return (
        data: (await image.toByteData(format: ui.ImageByteFormat.rawRgba))!,
        width: image.width,
        height: image.height,
      );
    } finally {
      image.dispose();
    }
  }))!;
}

int _pixel(_Pixels image, int x, int y) {
  final offset = (y * image.width + x) * 4;
  return image.data.getUint32(offset);
}

int _colorCount(_Pixels image, Color color) {
  final argb = color.toARGB32();
  final rgba = ((argb & 0xFFFFFF) << 8) | (argb >> 24);
  var count = 0;
  for (var i = 0; i < image.data.lengthInBytes; i += 4) {
    if (image.data.getUint32(i) == rgba) count++;
  }
  return count;
}

void main() {
  Future<void> pumpEditor(
    WidgetTester tester,
    List<EditorNode> nodes, {
    MicaTokens tokens = MicaTokens.light,
    EditorCommandHook? hook,
  }) async {
    await tester.pumpWidget(
      MicaTheme(
        tokens: tokens,
        child: MaterialApp(
          theme: tokens.toMaterialTheme(),
          localizationsDelegates: AppLocalizations.localizationsDelegates,
          supportedLocales: AppLocalizations.supportedLocales,
          home: Scaffold(
            body: Padding(
              padding: const EdgeInsets.only(top: 100),
              child: RepaintBoundary(
                key: _canvasKey,
                child: MicaEditor(
                  rootBlockId: 'root',
                  nodes: nodes,
                  version: 0,
                  canEdit: true,
                  appearance: EditorAppearance(tokens: tokens),
                  onApplyOperations: (_) async {},
                  commandHook: hook,
                ),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pump();
  }

  testWidgets(
    'list selection formats via floating bar, keyboard and host hook',
    (tester) async {
      final hook = EditorCommandHook();
      await pumpEditor(tester, [
        for (var i = 0; i < 3; i++)
          EditorNode(id: 'item$i', kind: 'bulleted_list', text: 'list item $i'),
      ], hook: hook);
      await tester.tapAt(
        tester.getTopLeft(find.byType(MicaEditor)) + const Offset(55, 15),
      );
      await _selectAll(tester);
      expect(find.byIcon(Icons.format_bold), findsOneWidget);
      expect(find.byIcon(Icons.format_italic), findsOneWidget);
      await tester.tap(find.byIcon(Icons.format_bold));
      await tester.pumpAndSettle();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyI);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      hook.toggleMark('strike');
      await tester.pumpAndSettle();
      final nodes = tester
          .widget<DocumentSurface>(find.byType(DocumentSurface))
          .nodes;
      for (final node in nodes) {
        expect(
          marksFromData(node.data).map((mark) => mark.type),
          containsAll(['bold', 'italic', 'strike']),
        );
        expect(node.kind, 'bulleted_list');
      }
    },
  );

  testWidgets('a link dialog formats all selected list items', (tester) async {
    final hook = EditorCommandHook();
    await pumpEditor(tester, [
      for (var i = 0; i < 3; i++)
        EditorNode(id: 'item$i', kind: 'bulleted_list', text: 'item $i'),
    ], hook: hook);
    await tester.tapAt(
      tester.getTopLeft(find.byType(MicaEditor)) + const Offset(55, 15),
    );
    await _selectAll(tester);
    hook.editLink();
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'https://example.test');
    await tester.tap(
      find.descendant(
        of: find.byType(AlertDialog),
        matching: find.byType(FilledButton),
      ),
    );
    await tester.pumpAndSettle();
    final nodes = tester
        .widget<DocumentSurface>(find.byType(DocumentSurface))
        .nodes;
    for (final node in nodes) {
      expect(marksFromData(node.data).single.href, 'https://example.test');
    }
    expect(tester.takeException(), isNull);
  });

  for (final (name, tokens) in [
    ('light', MicaTokens.light),
    ('dark', MicaTokens.dark_),
  ]) {
    testWidgets('$name table editing has no inherited field frame', (
      tester,
    ) async {
      await pumpEditor(tester, [
        EditorNode(
          id: 't',
          kind: 'table',
          text: '',
          data: TableData([
            [TableCell('header'), TableCell('other')],
            [TableCell('value'), TableCell('body')],
          ]).toBlockData(),
        ),
      ], tokens: tokens);
      final render = tester.renderObject<RenderDocument>(
        find.byType(DocumentSurface),
      );
      final cell = render.tableCellRect(0, 1, 0)!;
      await tester.tapAt(render.localToGlobal(cell.center));
      await tester.pumpAndSettle();
      expect(find.byType(TextField), findsOneWidget);
      final decoration = tester
          .widget<InputDecorator>(find.byType(InputDecorator))
          .decoration;
      expect(decoration.enabledBorder, InputBorder.none);
      expect(decoration.focusedBorder, InputBorder.none);
      expect(decoration.filled, isFalse);
      expect(
        _colorCount(await _capture(tester), tokens.editor.caret),
        0,
        reason: 'editing a cell must not frame the whole table',
      );
      await tester.enterText(find.byType(TextField), 'edited');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      final saved = tester
          .widget<DocumentSurface>(find.byType(DocumentSurface))
          .nodes
          .single;
      expect(TableData.fromBlock(saved.data).rows[1][0], 'edited');
    });

    testWidgets(
      '$name selected table keeps its grid without an extra outline',
      (tester) async {
        await pumpEditor(tester, [
          EditorNode(
            id: 't',
            kind: 'table',
            text: '',
            data: TableData([
              [TableCell('header')],
              [TableCell('body')],
            ]).toBlockData(),
          ),
        ], tokens: tokens);
        final render = tester.renderObject<RenderDocument>(
          find.byType(DocumentSurface),
        );
        render.selection = const DocSelection.collapsed(DocPosition(0, 0));
        await tester.pump();
        final image = await _capture(tester);
        expect(_colorCount(image, tokens.editor.caret), 0);
        final cell = render.tableCellRect(0, 1, 0)!;
        expect(
          _pixel(image, cell.left.floor(), cell.center.dy.floor()),
          isNot(_pixel(image, cell.left.floor() + 3, cell.center.dy.floor())),
          reason: 'the normal grid line must remain visible',
        );
      },
    );
  }
}
