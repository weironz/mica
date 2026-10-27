// Production dart2js M2 benchmark. Compile from the Flutter package with
// flutter build web --release --target=../../scripts/md-wasm-bench.dart
// --output=build/md_wasm_bench --no-web-resources-cdn.
import 'dart:convert';
import 'dart:js_interop';

import 'package:flutter/material.dart';
import 'package:mica_flutter/editor/markdown.dart' show markdownToBlocks;
import 'package:mica_flutter/markdown/markdown_codec.dart';

import '../clients/mica_flutter/test/support/markdown_bench_inputs.dart';

@JS('micaMd')
external _Engine get _engine;
@JS('runMdM2')
external set _run(JSFunction value);
@JS('mdM2Diff')
external set _diff(JSFunction value);

extension type _Engine(JSObject _) implements JSObject {
  external String parseBody(String markdown, bool fragment);
  external String engineInfo();
}

int _sink = 0;

// Measure the actual public facade, including its error boundary and shared
// markdown_wire decoder. The direct wire call below is only a byte diagnostic.
List<BlockSpec> _full(String source) => markdownCodec.parseBlocks(source);

String _repeat(String sample, int length) {
  final buffer = StringBuffer();
  while (buffer.length < length) {
    buffer.write(sample);
  }
  return buffer.toString().substring(0, length);
}

// Work is consumed so the compiler cannot eliminate either call path. Result
// hashing/equivalence is outside timed batches; per-call consumption is O(1).
double _batch(List<BlockSpec> Function() run, int count) {
  final watch = Stopwatch()..start();
  for (var i = 0; i < count; i++) {
    final blocks = run();
    _sink = (_sink + blocks.length + blocks.first.text.length) & 0x3fffffff;
  }
  watch.stop();
  return watch.elapsedMicroseconds / 1000 / count;
}

double _median(List<double> values) {
  final sorted = [...values]..sort();
  return sorted[sorted.length ~/ 2];
}

String _canonical(List<BlockSpec> blocks) => jsonEncode([
  for (final block in blocks)
    {'type': block.kind, 'text': block.text, 'data': block.data},
]);

JSString _measure() {
  final cases = [
    (name: 's0.87', source: hotPath87, count: 1024),
    (name: 's0.10k', source: _repeat(markdownFixtureSample, 10000), count: 8),
    (name: 's0.100k', source: _repeat(markdownFixtureSample, 100000), count: 1),
  ];
  final results = <Object>[];
  for (final input in cases) {
    final full = _full(input.source);
    final dart = markdownToBlocks(input.source);
    // JSON key ordering is immaterial; canonicalize recursively for comparison.
    Object? sorted(Object? value) {
      if (value is Map) {
        final keys = value.keys.cast<String>().toList()..sort();
        return {for (final key in keys) key: sorted(value[key])};
      }
      if (value is List) return value.map(sorted).toList();
      return value;
    }

    final fullCanonical = jsonEncode(sorted(jsonDecode(_canonical(full))));
    final dartCanonical = jsonEncode(sorted(jsonDecode(_canonical(dart))));
    final rounds = <Object>[];
    for (var round = 0; round < 3; round++) {
      _batch(() => _full(input.source), input.count);
      _batch(() => markdownToBlocks(input.source), input.count);
      final fullSamples = <double>[];
      final dartSamples = <double>[];
      for (var pair = 0; pair < 15; pair++) {
        void a() =>
            fullSamples.add(_batch(() => _full(input.source), input.count));
        void b() => dartSamples.add(
          _batch(() => markdownToBlocks(input.source), input.count),
        );
        if ((pair + round).isEven) {
          a();
          b();
        } else {
          b();
          a();
        }
      }
      final fullMs = _median(fullSamples);
      final dartMs = _median(dartSamples);
      rounds.add({
        'full_ms': fullSamples,
        'dart_ms': dartSamples,
        'full_median_ms': fullMs,
        'dart_median_ms': dartMs,
        'ratio': fullMs / dartMs,
      });
    }
    results.add({
      'name': input.name,
      'utf16_length': input.source.length,
      'batch': input.count,
      'full_blocks': full.length,
      'dart_blocks': dart.length,
      'equivalent': fullCanonical == dartCanonical,
      'rounds': rounds,
    });
  }
  return jsonEncode({
    'engine': jsonDecode(_engine.engineInfo()),
    'facade_engine': {
      'crate_version': markdownCodec.engineInfo.crateVersion,
      'schema_version': markdownCodec.engineInfo.schemaVersion,
      'source_fingerprint': markdownCodec.engineInfo.sourceFingerprint,
    },
    'path': 'MarkdownCodec.parseBlocks -> _WasmMarkdownCodec -> markdown_wire.decodeBlocks',
    'method': 'production dart2js; real public facade including wire JSON + JS string + Dart jsonDecode + BlockSpec; AB/BA 15 pairs x3 rounds',
    'sink': _sink,
    'cases': results,
  }).toJS;
}

JSString _differences() => jsonEncode([
  for (final length in [87, 10000, 100000])
    {
      'id': 's0.$length',
      'source': length == 87
          ? hotPath87
          : _repeat(markdownFixtureSample, length),
      'wire': _engine.parseBody(
        length == 87 ? hotPath87 : _repeat(markdownFixtureSample, length),
        false,
      ),
      'dart': jsonDecode(
        _canonical(
          markdownToBlocks(
            length == 87 ? hotPath87 : _repeat(markdownFixtureSample, length),
          ),
        ),
      ),
    },
]).toJS;

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await initMarkdownCodec();
  // Fail the benchmark if startup installed the unavailable codec: measurements
  // must never silently use a stub or a local replacement for the facade.
  final info = markdownCodec.engineInfo;
  if (info.sourceFingerprint.isEmpty) {
    throw StateError('MarkdownCodec did not report an engine fingerprint');
  }
  _run = _measure.toJS;
  _diff = _differences.toJS;
  runApp(
    const MaterialApp(
      home: Scaffold(body: Center(child: Text('M2 benchmark ready'))),
    ),
  );
}
