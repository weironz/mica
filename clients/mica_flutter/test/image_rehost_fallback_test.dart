import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

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
    Future<Uint8List?> Function(String)? loadBytes,
    Future<({String fileId, String name})?> Function(
      Uint8List,
      String,
      String,
    )? upload,
    List<Map<String, dynamic>>? ops,
    // Whether the automatic on-open re-host pass runs (production default is
    // ON). Off by default here so the menu-driven cases can exercise the
    // MANUAL recovery path in isolation; the one on-open case flips it true.
    bool reHost = false,
  }) async {
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
            // The automatic on-open pass (_rehostExternalImages, wired into
            // initState) runs the SAME server-then-client ladder. When the
            // client can read the bytes it silently converts the block to a
            // file_id — so if it were left on, the menu-driven cases would find
            // no external image to offer the "转存到 Mica 存储" action on, and
            // the plain-load case would see an unexpected re-host in flight.
            // Default it OFF to isolate those paths; the on-open case sets it on.
            reHostImages: reHost,
            onApplyOperations: (batch) async => ops?.addAll(batch),
            onImportImageUrl: import,
            onUploadImage: upload,
            onLoadImageBytes: (key) async {
              fetched.add(key);
              if (loadBytes != null) return loadBytes(key);
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
      // With the on-open pass off (pump default), no re-host is in flight for
      // this url, so _requestImage must FETCH it rather than wait on a file_id
      // that will never come. The guard asserts nothing re-hosts here.
      import: (_) async => fail('must not re-host with the on-open pass off'),
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

  // Opening a doc that arrived carrying an external link (import/sync) must
  // re-host it with no user action — the on-open pass wired into initState
  // (a7c93c3). Server-side import 403s hosts that block its datacenter IP, so
  // the fix is that THIS client, which can read the bytes, does the fetch.
  testWidgets('opening a doc auto-re-hosts an external image, no user action',
      (tester) async {
    final ops = <Map<String, dynamic>>[];
    await pump(
      tester,
      reHost: true, // production default: the on-open pass runs
      nodes: [
        EditorNode(id: 'i', kind: 'image', text: '', data: {'url': url}),
      ],
      ops: ops,
      // Server can reach this host — the on-open pass takes the server rung and
      // never has to fall through to the client.
      import: (_) async => (fileId: 'f-open', name: 'pic.png'),
      fetched: <String>[],
    );
    await tester.pump(const Duration(milliseconds: 50));
    final updated = ops.lastWhere((o) => o['type'] == 'update_block');
    expect((updated['data'] as Map)['file_id'], 'f-open',
        reason: 'the link is converted on open, before any right-click');
    expect((updated['data'] as Map).containsKey('url'), isFalse);
  });

  // The re-host ladder. Server first; when it can't reach the host (routine
  // for a CN-hosted server vs medium/imgur/…) the CLIENT does it, since it
  // demonstrably can read the bytes. Only when BOTH fail does the block stay
  // on its url — never silently, and never losing the link.
  group('client-side re-host fallback (via the image menu)', () {
    final png = Uint8List.fromList([1, 2, 3, 4]);

    // Right-click the image and pick the re-host entry — the user's recovery
    // path, and the same ladder the automatic post-paste pass runs.
    Future<void> rehostViaMenu(WidgetTester tester) async {
      final origin = tester.getTopLeft(find.byType(MicaEditor));
      final g = await tester.startGesture(
        origin + const Offset(60, 60),
        kind: PointerDeviceKind.mouse,
        buttons: kSecondaryButton,
      );
      await g.up();
      await tester.pumpAndSettle();
      expect(find.text('转存到 Mica 存储'), findsOneWidget,
          reason: 'an external image must offer the recovery action');
      await tester.tap(find.text('转存到 Mica 存储'));
      await tester.pumpAndSettle();
    }

    testWidgets('server fails, client fetches + uploads -> block gets a file_id',
        (tester) async {
      final uploads = <({String name, String mime})>[];
      final ops = <Map<String, dynamic>>[];
      await pump(
        tester,
        nodes: [
          EditorNode(id: 'i', kind: 'image', text: '', data: {'url': url}),
        ],
        ops: ops,
        import: (_) async => null, // server has no route to the host
        loadBytes: (_) async => png, // but WE can read it
        upload: (bytes, name, mime) async {
          uploads.add((name: name, mime: mime));
          return (fileId: 'f-new', name: name);
        },
        fetched: <String>[],
      );
      await rehostViaMenu(tester);

      expect(uploads, hasLength(1));
      expect(uploads.single.name, 'pic.png',
          reason: 'the name comes from the url path');
      expect(uploads.single.mime, 'image/png');
      final updated = ops.lastWhere((o) => o['type'] == 'update_block');
      expect((updated['data'] as Map)['file_id'], 'f-new');
      expect((updated['data'] as Map).containsKey('url'), isFalse,
          reason: 'once stored, the block no longer depends on the link');
    });

    testWidgets('server AND client both fail -> the url survives, no data loss',
        (tester) async {
      var uploaded = false;
      final ops = <Map<String, dynamic>>[];
      final nodes = [
        EditorNode(id: 'i', kind: 'image', text: '', data: {'url': url}),
      ];
      await pump(
        tester,
        nodes: nodes,
        ops: ops,
        import: (_) async => null,
        loadBytes: (_) async => null, // we can't read it either (dead link)
        upload: (_, __, ___) async {
          uploaded = true;
          return null;
        },
        fetched: <String>[],
      );
      await rehostViaMenu(tester);

      expect(uploaded, isFalse, reason: 'nothing to upload without bytes');
      expect(nodes.first.data['url'], url,
          reason: 'the link must survive: export still emits ![](url)');
      expect(ops.where((o) => o['type'] == 'update_block'), isEmpty,
          reason: 'a failed re-host must not rewrite the block');
    });

    testWidgets('a stored image offers no re-host entry and names its home',
        (tester) async {
      await pump(
        tester,
        nodes: [
          EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'}),
        ],
        import: (_) async => null,
        fetched: <String>[],
      );
      final origin = tester.getTopLeft(find.byType(MicaEditor));
      final g = await tester.startGesture(
        origin + const Offset(60, 60),
        kind: PointerDeviceKind.mouse,
        buttons: kSecondaryButton,
      );
      await g.up();
      await tester.pumpAndSettle();
      expect(find.text('已存储到 Mica · 链接公开可访问'), findsOneWidget);
      expect(find.text('转存到 Mica 存储'), findsNothing);
    });
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
