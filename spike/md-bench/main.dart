// P0 only: Flutter release wrapper around the REAL Mica application.
// flutter build web --release --no-web-resources-cdn -t ../../spike/md-bench/main.dart
// No production Dart source is changed. Playwright calls runMdP0() after boot.
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'package:mica_flutter/main.dart' as app;
import 'package:mica_flutter/editor/markdown.dart' show markdownToBlocks;

@JS('micaMdReady')
external JSPromise<JSAny?> get _ready;
@JS('micaMd')
external _MicaMd get _md;
@JS('performance.now')
external double _now();

// Prototype of the web codec in architecture §5. Input string -> wasm ->
// JSON string -> Dart jsonDecode -> BlockSpec (same consumer shape as Dart).
extension type _MicaMd._(JSObject _) implements JSObject {
  external String parseBody(String markdown);
}

void main() {
  globalContext['runMdP0'] = (() => _run().toJS).toJS;
  app.main();
}

const smallDoc =
    'Hello **world** and *friends*, see `code` [link](https://example.com). Plain text test.';

String buildDoc(int length) {
  const block = '## Section\n\n'
      'Some **bold**, *italic*, `code`, and a [link](https://example.com) '
      'in a paragraph of ordinary prose.\n\n'
      '- item one\n- item two with ~~strike~~\n\n> quoted line\n\n'
      '```dart\nfinal x = 1;\n```\n\n'
      '| a | b |\n| --- | --- |\n| 1 | 2 |\n\n';
  return List.filled((length / block.length).ceil(), block).join().substring(0, length);
}

int sink = 0;
int wasmFull(String src) {
  final rows = jsonDecode(_md.parseBody(src)) as List<dynamic>;
  final blocks = rows.map((row) {
    final b = row as Map<String, dynamic>;
    return (kind: b['type'] as String, text: b['text'] as String,
      data: b['data'] as Map<String, dynamic>);
  }).toList();
  return blocks.length + (blocks.isEmpty ? 0 : blocks.last.text.length);
}
int dartFull(String src) {
  final b = markdownToBlocks(src);
  return b.length + (b.isEmpty ? 0 : b.last.text.length);
}

double batch(int Function() call, int n) {
  final start = _now();
  for (var i = 0; i < n; i++) { sink += call(); }
  return (_now() - start) / n;
}
int calibrate(int Function() call) {
  var n = 1;
  while (n < 8192) {
    if (batch(call, n) * n >= 15) return n;
    n *= 2;
  }
  return n;
}
Map<String, Object> stats(List<double> samples, int batchSize) {
  samples.sort();
  return {'medianMs': samples[samples.length ~/ 2],
    'p95Ms': samples[(samples.length * .95).floor()],
    'samples': samples, 'batchSize': batchSize};
}

Future<JSString> _run() async {
  await _ready.toDart.timeout(const Duration(seconds: 10));
  if (smallDoc.length != 87) throw StateError('short input ${smallDoc.length} != 87');
  final results = <String, Object>{};
  final docs = {'s87': smallDoc, 's10k': buildDoc(10000), 's100k': buildDoc(100000)};
  for (final e in docs.entries) {
    final src = e.value;
    final firstStart = _now();
    final firstWasm = wasmFull(src);
    final firstWasmMs = _now() - firstStart;
    final firstDartStart = _now();
    final firstDart = dartFull(src);
    final firstDartMs = _now() - firstDartStart;
    for (var i = 0; i < 20; i++) { sink += wasmFull(src) + dartFull(src); }
    final nw = calibrate(() => wasmFull(src));
    final nd = calibrate(() => dartFull(src));
    final ws = <double>[], ds = <double>[];
    // Alternate order across samples to avoid systematic first-path bias.
    for (var i = 0; i < 15; i++) {
      if (i.isEven) {
        ws.add(batch(() => wasmFull(src), nw));
        ds.add(batch(() => dartFull(src), nd));
      } else {
        ds.add(batch(() => dartFull(src), nd));
        ws.add(batch(() => wasmFull(src), nw));
      }
      await Future<void>.delayed(Duration.zero);
    }
    final w = stats(ws, nw), d = stats(ds, nd);
    results[e.key] = {'characters': src.length,
      'firstWasmMs': firstWasmMs, 'firstDartMs': firstDartMs,
      'firstChecksum': [firstWasm, firstDart],
      'wasmJsonDecode': w, 'dart': d,
      'ratio': (w['medianMs'] as double) / (d['medianMs'] as double),
      'jsonBytes': utf8.encode(_md.parseBody(src)).length,
      'blocks': (jsonDecode(_md.parseBody(src)) as List).length};
  }
  return jsonEncode({'ok': true, 'results': results, 'sink': sink}).toJS;
}
