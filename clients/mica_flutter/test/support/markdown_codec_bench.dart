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
  var checksum = 0;
  // Equal untimed warmup before paired measurements so VM tiering does not
  // compare one cold implementation against the other's optimized loop.
  for (var i = 0; i < 3000; i++) {
    mirror.markdownToBlocks(hotPath87);
    codec.parseBlocks(hotPath87);
  }
  Map<String, Object> pair(
    String item,
    void Function() dart,
    void Function() rust,
    int batch, {
    double? budgetRatio,
    double? budgetMs,
  }) {
    assert(budgetRatio == null || budgetMs == null);
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
    };
    if (budgetRatio != null) {
      row['budget_ratio'] = budgetRatio;
      row['passes'] = ratios.every((r) => r <= budgetRatio);
    } else if (budgetMs != null) {
      row['budget_ms'] = budgetMs;
      row['passes'] = codecTimes.every((t) => t <= budgetMs);
    } else {
      // 02 §7 gates a clipboard copy by its COMBINED absolute time (copy.10k
      // below); one half's Dart ratio is recorded, not judged.
      row['passes'] = true;
    }
    rows.add(row);
    // ignore: avoid_print
    print(jsonEncode(row));
    return row;
  }

  // 02 §7: pasting 10k/100k is judged against Dart ×1.2; an 87-char parse is a
  // single boundary crossing, budgeted absolutely — at ~0.016 ms the Dart side
  // is mostly timer floor, so the ratio measures the FFI toll, not the engine.
  pair(
    'parse.87',
    () {
      final result = mirror.markdownToBlocks(hotPath87);
      checksum ^=
          result.length + (result.isEmpty ? 0 : result.first.text.length);
    },
    () {
      final result = codec.parseBlocks(hotPath87);
      checksum ^=
          result.length + (result.isEmpty ? 0 : result.first.text.length);
    },
    256,
    budgetMs: 0.1,
  );
  for (final (label, text, batch) in [
    ('10k', repeatTo(10000), 8),
    ('100k', repeatTo(100000), 2),
  ]) {
    pair(
      'parse.$label',
      () {
        final result = mirror.markdownToBlocks(text);
        checksum ^=
            result.length + (result.isEmpty ? 0 : result.first.text.length);
      },
      () {
        final result = codec.parseBlocks(text);
        checksum ^=
            result.length + (result.isEmpty ? 0 : result.first.text.length);
      },
      batch,
      budgetRatio: 1.2,
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
    final serialize = pair(
      'serialize.10k',
      () {
        checksum ^= controller.documentMarkdown().length;
      },
      () {
        checksum ^= codec.serializeBlocks(blocks).length;
      },
      8,
    );
    final html = pair(
      'html.10k',
      () {
        checksum ^= controller.selectionHtml().length;
      },
      () {
        checksum ^= codec.renderClipboardHtml(blocks).length;
      },
      8,
    );
    // 02 §7: copying a 10k selection is one frame for BOTH halves together,
    // desktop and web; the ratio to Dart is recorded, never a gate.
    final copyTimes = [
      for (var round = 0; round < 3; round++)
        (serialize['codec_ms']! as List<double>)[round] +
            (html['codec_ms']! as List<double>)[round],
    ];
    final copy = <String, Object>{
      'item': 'copy.10k',
      'codec_ms': copyTimes,
      'budget_ms': 16,
      'passes': copyTimes.every((t) => t <= 16),
    };
    rows.add(copy);
    // ignore: avoid_print
    print(jsonEncode(copy));
  } finally {
    controller.dispose();
  }
  final inlineMs = median(() {
    checksum ^= codec.parseInline(hotPath87).text.length;
  }, 256);
  rows.add({
    'item': 'boundary.inline87',
    'codec_ms': inlineMs,
    'budget_ms': 0.1,
    'passes': inlineMs <= 0.1,
  });
  // ignore: avoid_print
  print(jsonEncode({'consumed_checksum': checksum}));
  return rows;
}
