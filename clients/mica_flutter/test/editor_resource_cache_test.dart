import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/preview_raster.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

// Real layout/cache owners; only the expensive math producer is replaced.
// Bounded frames avoid hanging on the feedback loop under test.
class _CountingMathPreviewer extends RasterPreviewer {
  final calls = <String, int>{};
  int get total => calls.values.fold(0, (sum, count) => sum + count);
  @override
  String get id => 'math';
  @override
  Future<ui.Image?> produce(String source, double width, MicaTokens tokens) {
    calls.update(source, (n) => n + 1, ifAbsent: () => 1);
    final recorder = ui.PictureRecorder();
    Canvas(recorder).drawRect(const Rect.fromLTWH(0, 0, 36, 24), Paint());
    final picture = recorder.endRecording();
    final image = picture.toImageSync(36, 24);
    picture.dispose();
    return Future<ui.Image?>.value(image);
  }
}

class _PreviewHost extends StatefulWidget {
  const _PreviewHost({super.key, required this.nodes, required this.producer});
  final List<EditorNode> nodes;
  final RasterPreviewer producer;
  @override
  State<_PreviewHost> createState() => _PreviewHostState();
}

class _PreviewHostState extends State<_PreviewHost> {
  final blink = ValueNotifier(false);
  late final pipeline = RasterPreviewPipeline(
    previewers: [widget.producer],
    requestRebuild: (fn) {
      if (mounted) setState(fn);
    },
  );
  @override
  Widget build(BuildContext context) => Stack(
    children: [
      DocumentSurface(
        nodes: widget.nodes,
        contentRevision: 0,
        selection: null,
        showCaret: false,
        caretBlink: blink,
        appearance: const EditorAppearance(),
        previewImages: pipeline.images,
        previewBaselines: pipeline.baselines,
        onRequestPreview: pipeline.request,
        onResourceDemand: (demand) => pipeline.setDemand(demand.previews),
        onPreviewPainted: pipeline.touch,
      ),
      Positioned(left: -100000, top: 0, child: pipeline.offstageHost()),
    ],
  );
  @override
  void dispose() {
    pipeline.dispose();
    blink.dispose();
    super.dispose();
  }
}

Widget _viewport(Widget child, {double height = 240, ScrollController? controller}) => MaterialApp(
  localizationsDelegates: AppLocalizations.localizationsDelegates,
  supportedLocales: AppLocalizations.supportedLocales,
  locale: const Locale('zh'),
  home: Scaffold(
    body: Align(
      alignment: Alignment.topLeft,
      child: SizedBox(
        width: 360,
        height: height,
        child: SingleChildScrollView(controller: controller, child: child),
      ),
    ),
  ),
);
String _formula(int i) => 'x_{$i}';
List<EditorNode> _mathNodes(int count, {bool block = false}) => [
  for (var i = 0; i < count; i++)
    EditorNode(
      id: 'math-$i',
      kind: block ? 'math_block' : 'paragraph',
      text: _formula(i),
      data: block
          ? {}
          : {
              'marks': marksToJson([Mark(0, _formula(i).length, 'math')]),
            },
    ),
];
List<EditorNode> _denseMathNode(int count) {
  final text = StringBuffer();
  final marks = <Mark>[];
  for (var i = 0; i < count; i++) {
    if (i > 0) text.write(' ');
    final start = text.length;
    text.write(_formula(i));
    marks.add(Mark(start, text.length, 'math'));
  }
  return [
    EditorNode(
      id: 'dense',
      kind: 'paragraph',
      text: text.toString(),
      data: {'marks': marksToJson(marks)},
    ),
  ];
}

Future<void> _frames(
  WidgetTester tester,
  int count, {
  bool decode = false,
}) async {
  for (var i = 0; i < count; i++) {
    if (decode)
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 4)),
      );
    await tester.pump(const Duration(milliseconds: 16));
  }
}

typedef _Sample = ({
  int warmCalls,
  int finalCalls,
  int warmLayouts,
  int finalLayouts,
  int unique,
  int farCalls,
});
Future<_Sample> _mathSample(
  WidgetTester tester,
  List<EditorNode> nodes,
  int count, {
  double height = 240,
}) async {
  final producer = _CountingMathPreviewer();
  try {
    await tester.pumpWidget(
      _viewport(
        _PreviewHost(nodes: nodes, producer: producer),
        height: height,
      ),
    );
    final render = tester.renderObject<RenderDocument>(
      find.byType(DocumentSurface),
    );
    await _frames(tester, 12);
    final warmCalls = producer.total;
    final warmLayouts = render.debugLayoutCount;
    await _frames(tester, 12);
    return (
      warmCalls: warmCalls,
      finalCalls: producer.total,
      warmLayouts: warmLayouts,
      finalLayouts: render.debugLayoutCount,
      unique: producer.calls.length,
      farCalls: producer.calls[_formula(count - 1)] ?? 0,
    );
  } finally {
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  }
}

final _png = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==',
);
Future<(_Sample, bool)> _imageSample(WidgetTester tester, int count, {double height = 240}) async {
  final calls = <String, int>{};
  int total() => calls.values.fold(0, (sum, n) => sum + n);
  try {
    await tester.pumpWidget(
      _viewport(
        MicaEditor(
          rootBlockId: 'root',
          nodes: [
            for (var i = 0; i < count; i++)
              EditorNode(
                id: 'image-$i',
                kind: 'image',
                text: '',
                data: {'file_id': 'file-$i', 'width': 40},
              ),
          ],
          version: 0,
          canEdit: false,
          reHostImages: false,
          onApplyOperations: (_) async {},
          onLoadImageBytes: (key) async {
            calls.update(key, (n) => n + 1, ifAbsent: () => 1);
            return _png;
          },
        ),
        height: height,
      ),
    );
    final render = tester.renderObject<RenderDocument>(
      find.byType(DocumentSurface),
    );
    await _frames(tester, 24, decode: true);
    final warmCalls = total();
    final warmLayouts = render.debugLayoutCount;
    await _frames(tester, 12, decode: true);
    final sample = (
      warmCalls: warmCalls,
      finalCalls: total(),
      warmLayouts: warmLayouts,
      finalLayouts: render.debugLayoutCount,
      unique: calls.length,
      farCalls: calls['file-${count - 1}'] ?? 0,
    );
    // A stuck decode must not look idle: a decoded 40px image is shorter than its 180px placeholder.
    return (sample, render.debugBoxAt(0).$2 < 100);
  } finally {
    await tester.pumpWidget(const SizedBox());
    await tester.pump();
  }
}

void _expectIdle(_Sample sample, String label) {
  final message =
      '$label: calls ${sample.warmCalls}->${sample.finalCalls}; layouts ${sample.warmLayouts}->${sample.finalLayouts}; unique=${sample.unique}; far=${sample.farCalls}';
  debugPrint(message);
  expect(
    sample.warmCalls,
    greaterThan(0),
    reason: 'the probe must load resources',
  );
  expect(
    sample.finalCalls,
    sample.warmCalls,
    reason: 'fixed viewport must stop producing; $message',
  );
  expect(
    sample.finalLayouts,
    sample.warmLayouts,
    reason: 'settled geometry must stop relayout; $message',
  );
}

class _SolidOffstagePreviewer extends RasterPreviewer {
  @override
  String get id => 'math';
  @override
  Widget buildOffstage(String source, MicaTokens tokens) =>
      const SizedBox(width: 17, height: 11, child: ColoredBox(color: Colors.green));
}

void _additionalResourceTests() {
  testWidgets('scrolling back re-requests evicted rasters without relayout', (tester) async {
    final producer = _CountingMathPreviewer();
    final key = GlobalKey<_PreviewHostState>();
    final scroll = ScrollController();
    try {
      await tester.pumpWidget(_viewport(
        _PreviewHost(key: key, nodes: _mathNodes(160), producer: producer),
        controller: scroll,
      ));
      await _frames(tester, 12);
      final render = tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
      final firstX = render.caretRectFor(DocPosition(0, _formula(0).length))!.left;
      final firstCalls = producer.calls[_formula(0)]!;
      for (var i = 1; i <= 5; i++) {
        scroll.jumpTo(scroll.position.maxScrollExtent * i / 5);
        await _frames(tester, 12);
      }
      expect(key.currentState!.pipeline.imagesOf('math').containsKey(_formula(0)), isFalse,
          reason: 'the probe must actually evict the first raster');
      final layouts = render.debugLayoutCount;
      scroll.jumpTo(0);
      await _frames(tester, 12);
      expect(producer.calls[_formula(0)], greaterThan(firstCalls));
      expect(render.debugLayoutCount, layouts,
          reason: 'reloading a known-size texture must only repaint');
      expect(render.caretRectFor(DocPosition(0, _formula(0).length))!.left, closeTo(firstX, .01));
      final calls = producer.total;
      await _frames(tester, 12);
      expect(producer.total, calls);
    } finally {
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
      scroll.dispose();
    }
  });

  testWidgets('a long paragraph requests only nearby formula runs', (tester) async {
    final sample = await _mathSample(tester, _denseMathNode(600), 600, height: 120);
    expect(sample.unique, greaterThan(0));
    expect(sample.unique, lessThan(600));
    expect(sample.farCalls, 0, reason: 'node intersection must not enqueue every run');
  });

  testWidgets('a viewport containing more than 64 decoded images stays idle', (tester) async {
    tester.view.physicalSize = const Size(800, 6000);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final (sample, decoded) = await _imageSample(tester, 80, height: 6000);
    expect(decoded, isTrue);
    _expectIdle(sample, 'visible PNG / 80');
    expect(sample.unique, 80, reason: 'live demand may exceed the cold budget');
  });

  testWidgets('an offstage batch captures each source only once', (tester) async {
    final previous = ui.Image.onCreate;
    var captures = 0;
    ui.Image.onCreate = (image) {
      previous?.call(image);
      if (image.width == 34 && image.height == 22) captures++;
    };
    final key = GlobalKey<_PreviewHostState>();
    try {
      await tester.pumpWidget(_viewport(_PreviewHost(
        key: key, nodes: _mathNodes(12), producer: _SolidOffstagePreviewer(),
      )));
      for (var i = 0; i < 40 && key.currentState!.pipeline.imagesOf('math').length < 12; i++) {
        await _frames(tester, 1, decode: true);
      }
      expect(key.currentState!.pipeline.imagesOf('math'), hasLength(12));
      expect(captures, 12, reason: 'one request batch must not start parallel capture loops');
      await _frames(tester, 4, decode: true);
      expect(captures, 12);
    } finally {
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
      ui.Image.onCreate = previous;
    }
  });

  testWidgets('cold LRU eviction defers disposal and protects current demand', (tester) async {
    final producer = _CountingMathPreviewer();
    final pipeline = RasterPreviewPipeline(previewers: [producer], requestRebuild: (fn) => fn());
    try {
      pipeline.setDemand({for (var i = 0; i < 50; i++) (id: 'math', source: _formula(i)): 0});
      await _frames(tester, 4);
      expect(pipeline.imagesOf('math'), hasLength(50));
      final oldest = pipeline.imagesOf('math')[_formula(1)]!;
      pipeline.touch('math', _formula(0));
      pipeline.setDemand({});
      await tester.pump();
      expect(pipeline.imagesOf('math'), hasLength(48));
      expect(pipeline.imagesOf('math').containsKey(_formula(0)), isTrue);
      expect(oldest.debugDisposed, isFalse, reason: 'the retired frame is still alive this frame');
      await tester.pump();
      expect(oldest.debugDisposed, isTrue);
    } finally {
      pipeline.dispose();
      await tester.pump();
    }
  });

  testWidgets('reintroducing a cached source restores its pruned geometry', (tester) async {
    final producer = _CountingMathPreviewer();
    final key = GlobalKey<_PreviewHostState>();
    Future<void> build(List<EditorNode> nodes) => tester.pumpWidget(_viewport(
      _PreviewHost(key: key, nodes: nodes, producer: producer),
    ));
    try {
      await build(_mathNodes(1));
      await _frames(tester, 12);
      final render = tester.renderObject<RenderDocument>(find.byType(DocumentSurface));
      final x = render.caretRectFor(DocPosition(0, _formula(0).length))!.left;
      await build([EditorNode(id: 'plain', kind: 'paragraph', text: 'plain')]);
      await _frames(tester, 3);
      await build(_mathNodes(1));
      await _frames(tester, 4);
      expect(render.caretRectFor(DocPosition(0, _formula(0).length))!.left, closeTo(x, .01));
      expect(producer.total, 1);
    } finally {
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
    }
  });
}

void main() {
  _additionalResourceTests();
  testWidgets('inline math below the raster cap becomes idle', (tester) async {
    _expectIdle(await _mathSample(tester, _mathNodes(8), 8), 'inline / 8');
  });
  for (final block in [false, true]) {
    testWidgets(
      '${block ? 'block' : 'inline'} math beyond 48 sources becomes idle in a fixed viewport',
      (tester) async {
        final sample = await _mathSample(
          tester,
          _mathNodes(96, block: block),
          96,
        );
        _expectIdle(sample, '${block ? 'block' : 'inline'} / 96');
        expect(
          sample.farCalls,
          0,
          reason: 'far formulas must remain unrequested',
        );
      },
    );
  }
  testWidgets(
    'one visible paragraph with over 48 inline formulas becomes idle',
    (tester) async {
      final sample = await _mathSample(
        tester,
        _denseMathNode(80),
        80,
        height: 400,
      );
      _expectIdle(sample, 'dense inline / 80');
      expect(sample.unique, 80, reason: 'all visible formulas need previews');
    },
  );
  for (final count in [8, 96]) {
    testWidgets('decoded images / $count become idle in a fixed viewport', (
      tester,
    ) async {
      final (sample, decoded) = await _imageSample(tester, count);
      expect(decoded, isTrue, reason: 'real PNG decode must complete');
      _expectIdle(sample, 'PNG / $count');
      if (count > 64)
        expect(
          sample.farCalls,
          0,
          reason: 'far images must remain unrequested',
        );
    });
  }
}
