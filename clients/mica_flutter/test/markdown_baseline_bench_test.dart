// S0 performance baseline for the Dart Markdown mirror.
//
// The S2/S3/S4 performance budgets (docs/single-source/02-architecture.md §7)
// are relative to these numbers, so they must be measured before any code is
// deleted. Each item runs seven times; the first two are warmup and the median
// of the remaining five is reported as one JSON line:
//
//   {"platform":"vm","item":"parse.10k","median_ms":1.234}
//
// Run both platforms (the numbers are NOT comparable across them — this is
// dart2js versus the VM, which is exactly why both are recorded):
//
//   flutter test test/markdown_baseline_bench_test.dart
//   flutter test --platform chrome test/markdown_baseline_bench_test.dart
//
// It lives at the TOP LEVEL of `test/`, not `test/perf/` as the S0 plan says:
// this Flutter version's web test runner builds the entrypoint name from the
// path without the separator (`test/perf/x_test.dart` becomes
// `perfx_test.dart`), so a nested test 404s in Chrome and the run hangs
// forever at "loading". Reproduced with a one-line probe; see the S0 record.
//
// The inputs are identical on both platforms: Chrome tests cannot read files,
// so the conformance sample is embedded (`support/markdown_bench_inputs.dart`)
// and repeated to the target size. `markdown_bench_inputs_test.dart` proves the
// embedded copy still matches the fixture directory.
library;

import 'dart:convert';

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

import 'support/markdown_bench_inputs.dart';

void main() {
  final parse10k = _repeatTo(markdownFixtureSample, 10000);
  final parse100k = _repeatTo(markdownFixtureSample, 100000);
  final inlineSources = _inlineSources(500);
  final blocks10k = markdownToBlocks(parse10k);
  final controller = _controllerFor(blocks10k);

  tearDownAll(controller.dispose);

  test('hotPath87 input really is 87 code units', () {
    expect(hotPath87.length, 87);
  });

  test('parse.87', () => _bench('parse.87', () => markdownToBlocks(hotPath87)));
  test('parse.10k', () => _bench('parse.10k', () => markdownToBlocks(parse10k)));
  test(
    'parse.100k',
    () => _bench('parse.100k', () => markdownToBlocks(parse100k)),
  );
  test(
    'inline.cell×500',
    () => _bench('inline.cell×500', () {
      for (final source in inlineSources) {
        parseInline(source);
      }
    }),
  );
  test(
    'serialize.10k',
    () => _bench('serialize.10k', () => controller.documentMarkdown()),
  );
  test(
    'html.10k',
    () => _bench('html.10k', () => controller.selectionHtml()),
  );
}

void _bench(String item, void Function() run) {
  for (var i = 0; i < 2; i++) {
    run(); // warmup, discarded
  }
  final samples = <double>[];
  for (var i = 0; i < 7; i++) {
    final watch = Stopwatch()..start();
    run();
    watch.stop();
    samples.add(watch.elapsedMicroseconds / 1000);
  }
  samples.sort();
  final median = samples[samples.length ~/ 2];
  // ignore: avoid_print
  print(jsonEncode({
    'platform': kIsWeb ? 'chrome' : 'vm',
    'item': item,
    'median_ms': double.parse(median.toStringAsFixed(3)),
  }));
}

String _repeatTo(String sample, int target) {
  if (sample.isEmpty) return sample;
  final buffer = StringBuffer();
  while (buffer.length < target) {
    buffer.write(sample);
  }
  return buffer.toString().substring(0, target);
}

/// Inline sources: table cells when a line looks like a table row, otherwise
/// the line with its block marker stripped (heading, list, quote).
List<String> _inlineSources(int count) {
  final sources = <String>[];
  for (final raw in markdownFixtureSample.split('\n')) {
    final line = raw.trim();
    if (line.isEmpty || line.startsWith('```') || line.startsWith('~~~')) {
      continue;
    }
    if (line.contains('|')) {
      for (final cell in line.split('|')) {
        final trimmed = cell.trim();
        if (trimmed.isNotEmpty && !RegExp(r'^:?-+:?$').hasMatch(trimmed)) {
          sources.add(trimmed);
        }
      }
      continue;
    }
    var text = line;
    text = text.replaceFirst(RegExp(r'^#{1,6}\s+'), '');
    text = text.replaceFirst(RegExp(r'^>\s?'), '');
    text = text.replaceFirst(RegExp(r'^([-*+]|\d+[.)])\s+'), '');
    if (text.isNotEmpty) sources.add(text);
  }
  if (sources.isEmpty) sources.add('plain paragraph text');
  return [for (var i = 0; i < count; i++) sources[i % sources.length]];
}

EditorController _controllerFor(List<BlockSpec> blocks) {
  final controller = EditorController(rootBlockId: 'root', onOps: (_) async {});
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
  return controller;
}