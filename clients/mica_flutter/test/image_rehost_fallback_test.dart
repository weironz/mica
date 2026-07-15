import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';

// Pasted image urls are re-hosted server-side so links can't rot. But
// re-hosting is best-effort: a CN-hosted server routinely cannot reach the
// CDN the CLIENT can (medium/imgur/…). When it fails the editor must fall
// back to the original url — it used to skip fetching a url whenever
// re-hosting was on, so a failed re-host left the image blank forever, and a
// bare-url paste was dropped on the floor entirely.

void main() {
  Future<void> pump(
    WidgetTester tester, {
    required List<EditorNode> nodes,
    required Future<({String fileId, String name})?> Function(String) import,
    required List<String> fetched,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            onApplyOperations: (_) async {},
            onImportImageUrl: import,
            onLoadImageBytes: (key) async {
              fetched.add(key);
              return null; // decode failure is fine; we only assert the ATTEMPT
            },
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
  }

  const url = 'https://cdn.example.com/pic.png';

  testWidgets('a url image in a LOADED doc is fetched — no re-host is running '
      'for it, so waiting on a file_id would blank it forever', (tester) async {
    final fetched = <String>[];
    await pump(
      tester,
      nodes: [
        EditorNode(id: 'i', kind: 'image', text: '', data: {'url': url}),
      ],
      // Re-hosting only ever fires on paste; a synced/reopened doc never
      // calls it, so nothing is in flight for this url.
      import: (_) async => fail('must not re-host on plain load'),
      fetched: fetched,
    );
    await tester.pump(const Duration(milliseconds: 50));
    expect(fetched, contains(url));
  });

  testWidgets('a file_id image loads by file_id as usual', (tester) async {
    final fetched = <String>[];
    await pump(
      tester,
      nodes: [
        EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
      ],
      import: (_) async => null,
      fetched: fetched,
    );
    await tester.pump(const Duration(milliseconds: 50));
    expect(fetched, contains('f1'));
  });

  group('insertImage — url fallback keeps the paste', () {
    EditorController doc() {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
      c.load([EditorNode(id: 'p', kind: 'paragraph', text: '')]);
      c.selection = const DocSelection(
        anchor: DocPosition(0, 0),
        focus: DocPosition(0, 0),
      );
      return c;
    }

    test('inserting by url produces an image block on that url', () {
      final c = doc();
      c.insertImage(url: url);
      final img = c.nodes.firstWhere((n) => n.kind == 'image');
      expect(img.data['url'], url);
      expect(img.data['file_id'], isNull);
      expect(img.data['name'], 'pic.png', reason: 'name derives from the url');
    });

    test('inserting by fileId still carries file_id + name', () {
      final c = doc();
      c.insertImage(fileId: 'f1', name: 'shot.png');
      final img = c.nodes.firstWhere((n) => n.kind == 'image');
      expect(img.data['file_id'], 'f1');
      expect(img.data['name'], 'shot.png');
      expect(img.data['url'], isNull);
    });
  });
}
