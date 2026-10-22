// Issue #62(图片那一半):文档里的图片多于解码缓存上限(64)时,「驱逐 → 重新请求」死循环。
//
// 修复前:layout 对每个图片节点都请求;新图到达时**当场**按 LRU 驱逐,被驱逐的块退回
// 180px 占位、layout 再次请求它 → 再下载、再解码、再驱逐别的……页面在占位高度和真实高度
// 之间跳。这里装真的 MicaEditor(放在和应用里一样的 SingleChildScrollView 里),
// 让 onLoadImageBytes 返回真 PNG,数它被调了几次。
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

const int _count = 100; // > the editor's 64-image cap

void main() {
  testWidgets('more images than the cap: a steady state stops re-fetching', (
    tester,
  ) async {
    // A real 400×300 PNG: decodeCapped runs a real codec on it.
    final png = (await tester.runAsync(() async {
      final recorder = ui.PictureRecorder();
      Canvas(recorder).drawRect(
        const Rect.fromLTWH(0, 0, 400, 300),
        Paint()..color = const Color(0xFF3366CC),
      );
      final picture = recorder.endRecording();
      final img = await picture.toImage(400, 300);
      picture.dispose();
      final data = await img.toByteData(format: ui.ImageByteFormat.png);
      img.dispose();
      return data!.buffer.asUint8List();
    }))!;

    final loads = <String>[];
    final scroll = ScrollController();
    addTearDown(scroll.dispose);
    await tester.pumpWidget(
      MaterialApp(
        localizationsDelegates: AppLocalizations.localizationsDelegates,
        supportedLocales: AppLocalizations.supportedLocales,
        locale: const Locale('zh'),
        home: Scaffold(
          body: SingleChildScrollView(
            controller: scroll,
            child: MicaEditor(
              rootBlockId: 'root',
              nodes: [
                for (var i = 0; i < _count; i++)
                  EditorNode(
                    id: 'i$i',
                    kind: 'image',
                    text: '',
                    data: {'file_id': 'f$i'},
                  ),
              ],
              version: 0,
              canEdit: true,
              reHostImages: false,
              onApplyOperations: (_) async {},
              onLoadImageBytes: (key) async {
                loads.add(key);
                return Uint8List.fromList(png);
              },
            ),
          ),
        ),
      ),
    );

    // Real decodes finish on real time: give them some between frames.
    Future<void> frames(int n) async {
      for (var i = 0; i < n; i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 5)),
        );
        await tester.pump();
      }
    }

    double docHeight() => tester.getSize(find.byType(DocumentSurface)).height;

    Future<void> expectSteady(String where) async {
      await frames(40); // first-time loads settle
      final fetched = loads.length;
      final height = docHeight();
      await frames(40);
      expect(
        loads.length,
        fetched,
        reason:
            '$where: a steady state must not keep re-fetching images '
            '(evict → re-request loop)',
      );
      expect(
        docHeight(),
        height,
        reason: '$where: images must not jump between placeholder and size',
      );
    }

    await expectSteady('top of the document');
    // Every picture once, plus what the viewport takes back — not unbounded.
    expect(loads.length, lessThan(2 * _count));
    expect(
      docHeight(),
      greaterThan(_count * 300),
      reason: 'every block keeps its real 300px height, loaded or evicted',
    );

    // A quarter down: blocks that arrived while off-screen and were evicted
    // (the cap is 64 of 100). They kept their boxes; scrolling them into view
    // must fetch them back — which also proves eviction actually happened
    // here, so the steady state above is not a cache that never filled.
    final before = loads.length;
    scroll.jumpTo(scroll.position.maxScrollExtent / 4);
    await expectSteady('a quarter down');
    expect(
      loads.length,
      greaterThan(before),
      reason: 'evicted pictures that come on screen are fetched back',
    );
    final onScreen = [
      for (var i = 0; i < _count; i++)
        if (loads.lastIndexOf('f$i') >= before) 'f$i',
    ];
    expect(
      onScreen.length,
      lessThan(20),
      reason: 'only what the viewport (plus cull slack) shows is re-fetched',
    );
  });
}
