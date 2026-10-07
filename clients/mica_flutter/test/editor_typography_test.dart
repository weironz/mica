import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

void main() {
  late ValueNotifier<bool> blink;
  final boundaryKey = GlobalKey();
  setUp(() => blink = ValueNotifier(false));
  tearDown(() => blink.dispose());

  Future<RenderDocument> pump(
    WidgetTester tester,
    List<EditorNode> nodes, {
    EditorAppearance appearance = const EditorAppearance(),
    Map<String, Map<String, ui.Image>> previews = const {},
    Map<String, Map<String, double>> baselines = const {},
    Map<String, ui.Image> images = const {},
    double width = 400,
  }) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: RepaintBoundary(
            key: boundaryKey,
            child: SizedBox(
              width: width,
              child: DocumentSurface(
                nodes: nodes,
                contentRevision: 0,
                selection: null,
                showCaret: false,
                caretBlink: blink,
                appearance: appearance,
                previewImages: previews,
                previewBaselines: baselines,
                images: images,
              ),
            ),
          ),
        ),
      ),
    );
    return tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
  }

  Future<bool> paintsColor(WidgetTester tester, Color color) async {
    final boundary =
        boundaryKey.currentContext!.findRenderObject()!
            as RenderRepaintBoundary;
    return (await tester.runAsync(() async {
      final image = await boundary.toImage();
      final pixels = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
      image.dispose();
      final data = pixels!.buffer.asUint8List();
      final rgba = color.toARGB32();
      for (var i = 0; i < data.length; i += 4) {
        if (data[i] == ((rgba >> 16) & 255) &&
            data[i + 1] == ((rgba >> 8) & 255) &&
            data[i + 2] == (rgba & 255) &&
            data[i + 3] == ((rgba >> 24) & 255)) {
          return true;
        }
      }
      return false;
    }))!;
  }

  testWidgets('palette-only change repaints shaped text in the new color', (
    tester,
  ) async {
    final nodes = [EditorNode(id: 'p', kind: 'paragraph', text: 'MMMMMMMM')];
    await pump(tester, nodes);
    expect(await paintsColor(tester, MicaTokens.light.text.primary), isTrue);
    await pump(
      tester,
      nodes,
      appearance: const EditorAppearance(tokens: MicaTokens.dark_),
    );
    expect(
      await paintsColor(tester, MicaTokens.dark_.text.primary),
      isTrue,
      reason: 'a palette change must replace cached light glyph colors',
    );
    expect(await paintsColor(tester, MicaTokens.light.text.primary), isFalse);
  });

  testWidgets('inline formula and text use the same font scale exactly once', (
    tester,
  ) async {
    const source = r'\rightarrow';
    final nodes = [
      EditorNode(
        id: 'formula',
        kind: 'paragraph',
        text: source,
        data: {
          'marks': marksToJson([Mark(0, source.length, 'math')]),
        },
      ),
    ];
    final picture = ui.PictureRecorder();
    Canvas(picture).drawRect(const Rect.fromLTWH(0, 0, 72, 24), Paint());
    final recording = picture.endRecording();
    final image = recording.toImageSync(72, 24);
    recording.dispose();
    final previews = {
      'math': {source: image},
    };
    final r = await pump(tester, nodes, previews: previews);
    double formulaWidth() =>
        r.caretRectFor(const DocPosition(0, source.length))!.left -
        r.caretRectFor(const DocPosition(0, 0))!.left;
    final initial = formulaWidth();
    expect(initial, greaterThan(0));
    await pump(
      tester,
      nodes,
      previews: previews,
      appearance: const EditorAppearance(fontScale: 1.25),
    );
    expect(formulaWidth() / initial, closeTo(1.25, 0.001));
    await tester.pumpWidget(const SizedBox());
    image.dispose();
  });

  testWidgets('mutable raster, baseline and image caches invalidate geometry', (
    tester,
  ) async {
    ui.Image raster(int width, int height) {
      final recorder = ui.PictureRecorder();
      Canvas(recorder).drawRect(
        Rect.fromLTWH(0, 0, width.toDouble(), height.toDouble()),
        Paint(),
      );
      final picture = recorder.endRecording();
      final image = picture.toImageSync(width, height);
      picture.dispose();
      return image;
    }

    const source = r'\rightarrow';
    final first = raster(72, 24);
    final second = raster(144, 24);
    final photo = raster(100, 50);
    final taller = raster(100, 100);
    final nodes = [
      EditorNode(
        id: 'formula',
        kind: 'paragraph',
        text: source,
        data: {
          'marks': marksToJson([Mark(0, source.length, 'math')]),
        },
      ),
      EditorNode(
        id: 'image',
        kind: 'image',
        text: '',
        data: {'file_id': 'photo'},
      ),
    ];
    final previews = {
      'math': {source: first},
    };
    final baselines = {
      'math': {source: 4.0},
    };
    final images = {'photo': photo};
    final r = await pump(
      tester,
      nodes,
      previews: previews,
      baselines: baselines,
      images: images,
    );
    double formulaWidth() =>
        r.caretRectFor(const DocPosition(0, source.length))!.left -
        r.caretRectFor(const DocPosition(0, 0))!.left;
    double formulaTop() {
      for (var y = 0.0; y < 60; y += 0.25) {
        if (r.inlineMathAt(Offset(EditorTheme.gutter + 1, y)) != null) return y;
      }
      throw StateError('formula must be hit-testable');
    }

    final width = formulaWidth();
    previews['math']![source] = second;
    await pump(
      tester,
      nodes,
      previews: previews,
      baselines: baselines,
      images: images,
    );
    expect(formulaWidth(), closeTo(width * 2, 0.01));
    final top = formulaTop();
    baselines['math']![source] = 10;
    await pump(
      tester,
      nodes,
      previews: previews,
      baselines: baselines,
      images: images,
    );
    expect(
      formulaTop(),
      isNot(closeTo(top, 0.5)),
      reason: 'baseline-only correction must reposition the atom',
    );
    final height = r.debugBoxAt(1).$2;
    images['photo'] = taller;
    await pump(
      tester,
      nodes,
      previews: previews,
      baselines: baselines,
      images: images,
    );
    expect(r.debugBoxAt(1).$2, greaterThan(height));
    await tester.pumpWidget(const SizedBox());
    for (final image in [first, second, photo, taller]) {
      image.dispose();
    }
  });

  testWidgets('rendered heading gaps group the following paragraph', (
    tester,
  ) async {
    final nodes = [
      EditorNode(id: 'p', kind: 'paragraph', text: 'paragraph'),
      EditorNode(id: 'h', kind: 'heading', text: 'heading', data: {'level': 1}),
      EditorNode(id: 'after', kind: 'paragraph', text: 'after'),
      EditorNode(id: 'next', kind: 'paragraph', text: 'next'),
    ];
    final r = await pump(tester, nodes);
    double gap(int i) =>
        r.debugBoxAt(i).$1 - (r.debugBoxAt(i - 1).$1 + r.debugBoxAt(i - 1).$2);
    expect(gap(1), 28);
    expect(gap(2), 10);
    expect(gap(3), 12);
    final origin = r.caretRectFor(const DocPosition(2, 0))!;
    expect(r.positionAt(origin.center).node, 2);
  });

  testWidgets('font preference and narrow layout update live caret geometry', (
    tester,
  ) async {
    final nodes = [
      EditorNode(
        id: 'p',
        kind: 'paragraph',
        text: 'English text with several words to wrap 中文正文',
      ),
    ];
    final r = await pump(tester, nodes);
    final before = r.caretRectFor(const DocPosition(0, 12))!;
    await pump(
      tester,
      nodes,
      appearance: const EditorAppearance(fontScale: 17 / 16),
    );
    expect(
      r.caretRectFor(const DocPosition(0, 12))!.left,
      greaterThan(before.left),
    );
    await pump(
      tester,
      nodes,
      width: 160,
      appearance: const EditorAppearance(fontScale: 17 / 16),
    );
    final end = r.caretRectFor(DocPosition(0, nodes.single.text.length))!;
    expect(end.top, greaterThan(before.top));
    expect(r.positionAt(end.center).node, 0);
    expect(end.left, lessThanOrEqualTo(r.size.width));
  });
}
