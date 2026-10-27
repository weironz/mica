import 'dart:convert';

import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart' as mirror;
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'markdown_bench_inputs.dart';

/// Same code-unit slices and controller setup as the S0 benchmark. Run both
/// sides inside the same process/compiler mode; never divide VM by AOT times.
List<Map<String, Object>> measureMarkdownCodec(MarkdownCodec codec) {
  String repeatTo(int length) {
    final text = StringBuffer();
    while (text.length < length) {
      text.write(markdownFixtureSample);
    }
    return text.toString().substring(0, length);
  }

  double median(void Function() call, int batch) {
    final samples = <double>[];
    for (var sample = 0; sample < 7; sample++) {
      final watch = Stopwatch()..start();
      for (var i = 0; i < batch; i++) {
        call();
      }
      watch.stop();
      if (sample >= 2) samples.add(watch.elapsedMicroseconds / 1000 / batch);
    }
    samples.sort();
    return samples[2];
  }

  final rows = <Map<String, Object>>[];
  // Equal untimed warmup before paired measurements so VM tiering does not
  // compare one cold implementation against the other's optimized loop.
  for (var i = 0; i < 3000; i++) {
    mirror.markdownToBlocks(hotPath87);
    codec.parseBlocks(hotPath87);
  }
  void pair(
    String item,
    void Function() dart,
    void Function() rust,
    int batch,
  ) {
    // Alternate order across rounds to reduce first-run bias.
    final ratios = <double>[];
    final dartTimes = <double>[], codecTimes = <double>[];
    for (var round = 0; round < 3; round++) {
      late double d, c;
      if (round.isEven) {
        d = median(dart, batch);
        c = median(rust, batch);
      } else {
        c = median(rust, batch);
        d = median(dart, batch);
      }
      dartTimes.add(d);
      codecTimes.add(c);
      ratios.add(c / d);
    }
    final ratioOrder = [...ratios]..sort();
    final row = <String, Object>{
      'item': item,
      'dart_ms': dartTimes,
      'codec_ms': codecTimes,
      'ratios': ratios,
      'ratio_median': ratioOrder[1],
      'budget_ratio': 1.2,
      'passes': ratios.every((r) => r <= 1.2),
    };
    rows.add(row);
    // ignore: avoid_print
    print(jsonEncode(row));
  }

  for (final (label, text, batch) in [
    ('87', hotPath87, 256),
    ('10k', repeatTo(10000), 8),
    ('100k', repeatTo(100000), 2),
  ]) {
    pair(
      'parse.$label',
      () {
        mirror.markdownToBlocks(text);
      },
      () {
        codec.parseBlocks(text);
      },
      batch,
    );
  }
  final blocks = mirror.markdownToBlocks(repeatTo(10000));
  final controller = EditorController(rootBlockId: 'root', onOps: (_) async {});
  try {
    controller.load([
      for (var i = 0; i < blocks.length; i++)
        EditorNode(
          id: 'b$i',
          kind: blocks[i].kind,
          text: blocks[i].text,
          data: Map<String, dynamic>.from(blocks[i].data),
        ),
    ]);
    if (controller.nodes.isNotEmpty) {
      final last = controller.nodes.length - 1;
      controller.selection = DocSelection(
        anchor: const DocPosition(0, 0),
        focus: DocPosition(last, controller.nodes[last].text.length),
      );
    }
    pair(
      'serialize.10k',
      () {
        controller.documentMarkdown();
      },
      () {
        codec.serializeBlocks(blocks);
      },
      8,
    );
    pair(
      'html.10k',
      () {
        controller.selectionHtml();
      },
      () {
        codec.renderClipboardHtml(blocks);
      },
      8,
    );
  } finally {
    controller.dispose();
  }
  final inlineMs = median(() {
    codec.parseInline(hotPath87);
  }, 256);
  rows.add({
    'item': 'boundary.inline87',
    'codec_ms': inlineMs,
    'budget_ms': 0.1,
    'passes': inlineMs <= 0.1,
  });
  return rows;
}
