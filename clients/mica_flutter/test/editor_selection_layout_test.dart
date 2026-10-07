import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';

void main() {
  late ValueNotifier<bool> blink;
  ValueNotifier<Widget>? surface;
  setUp(() {
    blink = ValueNotifier(false);
    surface = null;
  });
  tearDown(() {
    surface?.dispose();
    blink.dispose();
  });

  Future<RenderDocument> pump(
    WidgetTester tester,
    List<EditorNode> nodes, {
    int? revision = 0,
    DocSelection? selection,
    double width = 320,
  }) async {
    final widget = SizedBox(
      width: width,
      child: DocumentSurface(
        nodes: nodes,
        contentRevision: revision,
        selection: selection,
        showCaret: false,
        caretBlink: blink,
        appearance: const EditorAppearance(),
      ),
    );
    if (surface == null) {
      surface = ValueNotifier(widget);
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: ValueListenableBuilder<Widget>(
              valueListenable: surface!,
              builder: (_, child, _) => child,
            ),
          ),
        ),
      );
    } else {
      surface!.value = widget;
      await tester.pump();
    }
    return tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
  }

  DocSelection caret(int node, int offset) =>
      DocSelection.collapsed(DocPosition(node, offset));

  testWidgets('ordinary caret and range notifications do not relayout', (
    tester,
  ) async {
    final nodes = List.generate(
      200,
      (i) => EditorNode(id: '$i', kind: 'paragraph', text: 'paragraph $i'),
    );
    final r = await pump(tester, nodes, selection: caret(0, 0));
    final before = r.debugLayoutCount;
    for (var i = 1; i < 8; i++) {
      await pump(tester, nodes, selection: caret(i, 3));
    }
    await pump(
      tester,
      nodes,
      selection: const DocSelection(
        anchor: DocPosition(1, 0),
        focus: DocPosition(5, 4),
      ),
    );
    expect(r.debugLayoutCount, before);
    expect(r.caretRectFor(const DocPosition(5, 4)), isNotNull);
  });

  testWidgets('content revision and list replacement invalidate live layout', (
    tester,
  ) async {
    final nodes = [EditorNode(id: 'a', kind: 'paragraph', text: 'short')];
    final r = await pump(tester, nodes);
    final initial = r.debugLayoutCount;
    nodes[0].text = 'longer\nwith another line';
    await pump(tester, nodes, revision: 1);
    expect(r.debugLayoutCount, greaterThan(initial));
    expect(r.debugTextAt(0), contains('another line'));
    final changed = r.debugLayoutCount;
    await pump(tester, [
      EditorNode(id: 'b', kind: 'paragraph', text: 'replacement'),
    ], revision: 1);
    expect(r.debugLayoutCount, greaterThan(changed));
    expect(r.debugTextAt(0), 'replacement');
  });

  testWidgets('callers without revisions still detect in-place edits', (
    tester,
  ) async {
    final nodes = [EditorNode(id: 'a', kind: 'paragraph', text: 'old')];
    final r = await pump(tester, nodes, revision: null);
    nodes[0].text = 'new text';
    await pump(tester, nodes, revision: null);
    expect(r.debugTextAt(0), 'new text');
  });

  testWidgets('code caret still scrolls horizontally with unchanged content', (
    tester,
  ) async {
    final source = 'a' * 150;
    final nodes = [EditorNode(id: 'code', kind: 'code_block', text: source)];
    final r = await pump(tester, nodes, selection: caret(0, 0));
    final before = r.debugLayoutCount;
    await pump(tester, nodes, selection: caret(0, source.length));
    expect(r.debugLayoutCount, greaterThan(before));
    final end = r.caretRectFor(DocPosition(0, source.length))!;
    expect(end.left, lessThanOrEqualTo(r.size.width));
    expect(end.left, greaterThan(EditorTheme.gutter));
  });

  testWidgets('details reveals selected hidden source and refolds on exit', (
    tester,
  ) async {
    for (final expanded in [false, true]) {
      final nodes = [
        EditorNode(id: 'before', kind: 'paragraph', text: 'before'),
        EditorNode(
          id: 'open',
          kind: 'code_block',
          text: '<details>\n<summary>Summary</summary>',
          data: {'raw': true, 'collapsed': !expanded},
        ),
        EditorNode(id: 'body', kind: 'paragraph', text: 'body'),
        EditorNode(
          id: 'close',
          kind: 'code_block',
          text: '</details>',
          data: {'raw': true},
        ),
        EditorNode(id: 'after', kind: 'paragraph', text: 'after'),
      ];
      final r = await pump(tester, nodes, selection: caret(4, 0));
      expect(r.debugDetailsHeaderAt(1), isNotNull);
      expect(r.debugHiddenAt(3), isTrue);
      await pump(tester, nodes, selection: caret(3, 0));
      expect(r.debugHiddenAt(3), isFalse);
      expect(r.debugDetailsHeaderAt(1), isNull);
      await pump(tester, nodes, selection: caret(4, 0));
      expect(r.debugDetailsHeaderAt(1), isNotNull);
      expect(r.debugHiddenAt(3), isTrue);
      expect(r.debugHiddenAt(2), !expanded);
    }
  });
}
