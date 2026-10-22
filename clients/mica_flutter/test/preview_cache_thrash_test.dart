// Issue #62:文档里的公式多于预览缓存上限(48)时,「驱逐 → 重新请求」死循环。
//
// 修复前:layout 对**每个**节点都请求栅格;缓存按**插入顺序**驱逐(可能正是屏幕上那个);
// 新栅格到达清空 layout cache → 重排 → 被驱逐的又被请求 → 再插入 → 再驱逐别的……
// 每一帧都在重新生产、文档高度在「源码形态 ↔ 排版形态」之间跳。
//
// 这里用真 RasterPreviewPipeline + 真 DocumentSurface,只把 producer 换成同步出图的
// 计数器(direct form),量「稳态下还在不在生产」。
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/preview_raster.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

/// Direct-form 'math' previewer that counts how often it is asked to produce.
class _CountingMath extends RasterPreviewer {
  int produced = 0;

  @override
  String get id => 'math';

  @override
  Future<ui.Image?>? produce(String source, double w, MicaTokens tokens) {
    produced++;
    final recorder = ui.PictureRecorder();
    Canvas(recorder).drawRect(const Rect.fromLTWH(0, 0, 80, 200), Paint());
    final picture = recorder.endRecording();
    // 80×200 px at mathPixelRatio 2 → a ~120px-tall block: ~10 on screen,
    // far below the cap, while the document holds twice the cap.
    final img = picture.toImageSync(80, 200);
    picture.dispose();
    return Future.value(img);
  }
}

const int _count = 100; // > 2 × the pipeline's 48 cap

void main() {
  testWidgets('more formulas than the cap: a steady state stops producing', (
    tester,
  ) async {
    final producer = _CountingMath();
    StateSetter? rebuild;
    final pipeline = RasterPreviewPipeline(
      previewers: [producer],
      requestRebuild: (fn) {
        fn();
        rebuild?.call(() {});
      },
    );
    addTearDown(pipeline.dispose);
    final blink = ValueNotifier(false);
    addTearDown(blink.dispose);
    final scroll = ScrollController();
    addTearDown(scroll.dispose);
    final nodes = [
      for (var i = 0; i < _count; i++)
        EditorNode(id: 'm$i', kind: 'math_block', text: 'x_{$i}'),
    ];

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: StatefulBuilder(
            builder: (context, setState) {
              rebuild = setState;
              return SingleChildScrollView(
                controller: scroll,
                child: DocumentSurface(
                  nodes: nodes,
                  selection: null,
                  showCaret: false,
                  caretBlink: blink,
                  appearance: const EditorAppearance(),
                  previewImages: pipeline.images,
                  previewBaselines: pipeline.baselines,
                  onRequestPreview: pipeline.request,
                ),
              );
            },
          ),
        ),
      ),
    );

    double docHeight() => tester.getSize(find.byType(DocumentSurface)).height;
    Future<void> frames(int n) async {
      for (var i = 0; i < n; i++) {
        await tester.pump();
      }
    }

    Future<void> expectSteady(String where) async {
      await frames(10); // let first-time production settle
      final produced = producer.produced;
      final height = docHeight();
      await frames(30);
      expect(
        producer.produced,
        produced,
        reason:
            '$where: a steady state must not keep re-producing rasters '
            '(evict → re-request loop)',
      );
      expect(
        docHeight(),
        height,
        reason: '$where: formulas must not flip between source and typeset',
      );
      // Nothing the viewport shows may be missing its pixels.
      final render = tester.renderObject<RenderDocument>(
        find.byType(DocumentSurface),
      );
      final top = scroll.offset;
      for (var i = 0; i < _count; i++) {
        final r = render.caretRectFor(DocPosition(i, 0));
        if (r == null || r.bottom < top || r.top > top + 600) continue;
        expect(
          pipeline.imagesOf('math').containsKey('x_{$i}'),
          isTrue,
          reason: '$where: on-screen formula $i was evicted',
        );
      }
      expect(
        pipeline.imagesOf('math').length,
        lessThanOrEqualTo(48),
        reason: '$where: the cap still holds when the screen fits in it',
      );
    }

    await expectSteady('top of the document');
    // Every formula typeset once, plus re-production of what the viewport
    // needs back — not an unbounded count.
    expect(producer.produced, lessThan(2 * _count));

    scroll.jumpTo(scroll.position.maxScrollExtent / 2);
    await expectSteady('middle of the document');
    scroll.jumpTo(0);
    await expectSteady('back at the top');
  });
}
