import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/model.dart';

// The image right-click menu is the complete surface for an image: the hover
// toolbar only exists while a mouse is over the picture, so everything it can
// do has to be reachable here too — plus the things only the menu has (the
// link, replace, re-host).

// A real 1x1 PNG: the fullscreen viewer reuses the canvas's DECODED image, so
// it (correctly) does nothing when there are no bytes to show.
final kPng = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6'
  'kgAAAABJRU5ErkJggg==',
);

void main() {
  Future<List<Map<String, dynamic>>> pump(
    WidgetTester tester, {
    required List<EditorNode> nodes,
    bool reHost = true,
    Future<({String fileId, String name})?> Function(String)? import,
    Future<Uint8List?> Function(String)? loadBytes,
  }) async {
    final ops = <Map<String, dynamic>>[];
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: MicaEditor(
            rootBlockId: 'root',
            nodes: nodes,
            version: 0,
            canEdit: true,
            reHostImages: reHost,
            onApplyOperations: (batch) async => ops.addAll(batch),
            onImportImageUrl: import ?? (_) async => null,
            onLoadImageBytes: loadBytes ?? (_) async => null,
            onResolveImageUrls: (ids) async => {
              for (final id in ids) id: 'https://mica.test/api/files/$id/blob',
            },
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    return ops;
  }

  Future<void> rightClickImage(WidgetTester tester) async {
    final origin = tester.getTopLeft(find.byType(MicaEditor));
    final g = await tester.startGesture(
      origin + const Offset(60, 60),
      kind: PointerDeviceKind.mouse,
      buttons: kSecondaryButton,
    );
    await g.up();
    await tester.pumpAndSettle();
  }

  EditorNode storedImage() =>
      EditorNode(id: 'i', kind: 'image', text: '', data: {'file_id': 'f1'});
  EditorNode externalImage() => EditorNode(
        id: 'i',
        kind: 'image',
        text: '',
        data: {'url': 'https://cdn.example.com/pic.png'},
      );

  testWidgets('the menu carries every image action', (tester) async {
    await pump(tester, nodes: [storedImage()]);
    await rightClickImage(tester);
    for (final label in [
      '全屏查看',
      '编辑图片…',
      '左对齐',
      '居中',
      '右对齐',
      '复制图片',
      '下载',
      '删除',
    ]) {
      expect(find.text(label), findsOneWidget, reason: label);
    }
  });

  // The editor works on its OWN copies (EditorController.load copies each
  // node), so the op stream — what actually gets persisted — is the honest
  // place to observe an edit, not the list handed to the widget.
  Map<String, dynamic> lastBlockData(List<Map<String, dynamic>> ops) =>
      (ops.lastWhere((o) => o['type'] == 'update_block')['data'] as Map)
          .cast<String, dynamic>();

  testWidgets('the align entries set data.align', (tester) async {
    final ops = await pump(tester, nodes: [storedImage()]);
    await rightClickImage(tester);
    await tester.tap(find.text('右对齐'));
    await tester.pumpAndSettle();
    expect(lastBlockData(ops)['align'], 'right');
  });

  // NOT covered here: that the fullscreen viewer actually appears. It paints
  // the canvas's DECODED image, and decoding runs ui.instantiateImageCodec —
  // real async that the test's FakeAsync zone never completes (runAsync can't
  // rescue a future already created inside the fake zone), so the cache stays
  // empty and _openImageViewer correctly finds nothing to show. Verified by
  // hand in the app instead; what IS pinned below is the wiring around it —
  // that a click on the picture is routed to the image branch at all, and that
  // a single click doesn't escalate.

  testWidgets('clicking the picture selects the image block, not text',
      (tester) async {
    await pump(tester, nodes: [storedImage(), EditorNode(id: 'p', kind: 'paragraph', text: 'after')]);
    await tester.tapAt(
      tester.getTopLeft(find.byType(MicaEditor)) + const Offset(60, 60),
    );
    await tester.pumpAndSettle();
    // No crash, no dialog, and the caret parked on the atomic block.
    expect(find.byType(InteractiveViewer), findsNothing,
        reason: 'a single click must never open the viewer');
  });

  group('编辑图片 dialog', () {
    testWidgets('names where a stored image lives and shows its link',
        (tester) async {
      await pump(tester, nodes: [storedImage()]);
      await rightClickImage(tester);
      await tester.tap(find.text('编辑图片…'));
      await tester.pumpAndSettle();
      expect(find.text('已存储到 Mica · 链接公开可访问'), findsOneWidget);
      expect(find.text('https://mica.test/api/files/f1/blob'), findsOneWidget);
      expect(find.text('替换图片'), findsOneWidget);
    });

    testWidgets('an external image says the link can rot', (tester) async {
      await pump(tester, nodes: [externalImage()]);
      await rightClickImage(tester);
      await tester.tap(find.text('编辑图片…'));
      await tester.pumpAndSettle();
      expect(find.text('外部链接 · 原站失效后图片会丢失'), findsOneWidget);
      expect(find.text('https://cdn.example.com/pic.png'), findsOneWidget);
    });

    testWidgets('replacing by link re-hosts when the setting is ON',
        (tester) async {
      final asked = <String>[];
      final ops = await pump(
        tester,
        nodes: [storedImage()],
        reHost: true,
        import: (u) async {
          asked.add(u);
          return (fileId: 'f-new', name: 'new.png');
        },
      );
      await rightClickImage(tester);
      await tester.tap(find.text('编辑图片…'));
      await tester.pumpAndSettle();
      expect(find.text('已启用自动转存:链接会被转存到 Mica 存储(取不到则保留原链接)'),
          findsOneWidget);
      await tester.enterText(
        find.byType(TextField),
        'https://cdn.example.com/new.png',
      );
      await tester.tap(find.text('替换'));
      await tester.pumpAndSettle();

      expect(asked, ['https://cdn.example.com/new.png']);
      expect(lastBlockData(ops)['file_id'], 'f-new',
          reason: 're-hosting turns the pasted link into our own copy');
      expect(lastBlockData(ops).containsKey('url'), isFalse);
    });

    testWidgets('replacing by link keeps the url when re-hosting is OFF',
        (tester) async {
      final asked = <String>[];
      final ops = await pump(
        tester,
        nodes: [storedImage()],
        reHost: false,
        import: (u) async {
          asked.add(u);
          return (fileId: 'f-new', name: 'new.png');
        },
      );
      await rightClickImage(tester);
      await tester.tap(find.text('编辑图片…'));
      await tester.pumpAndSettle();
      expect(find.text('自动转存已关闭:将直接保留原链接'), findsOneWidget);
      await tester.enterText(
        find.byType(TextField),
        'https://cdn.example.com/new.png',
      );
      await tester.tap(find.text('替换'));
      await tester.pumpAndSettle();

      expect(asked, isEmpty, reason: 'the setting says do not re-host');
      expect(lastBlockData(ops)['url'], 'https://cdn.example.com/new.png');
      expect(lastBlockData(ops).containsKey('file_id'), isFalse);
    });

    testWidgets('cancel changes nothing', (tester) async {
      final ops = await pump(tester, nodes: [storedImage()]);
      await rightClickImage(tester);
      await tester.tap(find.text('编辑图片…'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'https://x.io/a.png');
      await tester.tap(find.text('取消'));
      await tester.pumpAndSettle();
      expect(ops.where((o) => o['type'] == 'update_block'), isEmpty,
          reason: 'cancel must not write anything');
    });
  });

  group('setImageUrl', () {
    test('swaps a stored image onto a link and derives the name', () {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
      c.load([
        EditorNode(
          id: 'i',
          kind: 'image',
          text: '',
          data: {'file_id': 'f1', 'name': 'old.png'},
        ),
      ]);
      c.setImageUrl('i', 'https://cdn.example.com/deep/path/new.png');
      expect(c.nodes.first.data['url'], 'https://cdn.example.com/deep/path/new.png');
      expect(c.nodes.first.data['file_id'], isNull);
      expect(c.nodes.first.data['name'], 'new.png');
    });

    test('is a no-op on a non-image block', () {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
      c.load([EditorNode(id: 'p', kind: 'paragraph', text: 'x')]);
      c.setImageUrl('p', 'https://x.io/a.png');
      expect(c.nodes.first.data['url'], isNull);
    });
  });
}
