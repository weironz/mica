// P0 cost-attribution entrypoint (spike only, never merged).
//
// This is a REAL release Flutter web build of the mica_flutter package:
//   cd clients/mica_flutter
//   flutter build web --release --no-web-resources-cdn -t ../../spike/md-bench/main.dart
// It boots the actual app (app.main()) and exposes runMdP0() for Playwright.
// The Dart comparator is the production `markdownToBlocks`, compiled by the
// same dart2js release pass as the shipped app.
//
// Stages, each measured in isolation (no stage substitutes for another):
//   loopOverhead   empty loop, quantifies the timer/loop floor
//   wasmNoop       wasm call overhead, no arguments
//   echoMd/Json    full string in→out round trip through wasm (boundary)
//   parseOnly      engine parse, blocks consumed into a digest, no JSON
//   sequentialParse/uuidParse  matched parse API + same digest; only IDs differ
//   parseBody      parse + BlockJson mapping + Rust serde_json → JSON string
//   dartDecode     jsonDecode of pre-generated JSON
//   dartMap        JSON list → BlockSpec records
//   full           parseBody + jsonDecode + map (the production path)
//   dartMarkdown   the existing Dart mirror (same browser, same page)
//
// Equivalence is checked ONCE per input, outside the timed samples: the full
// Rust path and Dart mirror are canonicalised (ids/children dropped, marks
// sorted by start/end/type) and compared block-by-block.
import 'dart:async';
import 'dart:convert';
import 'dart:js_interop';
import 'dart:js_interop_unsafe';
import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:mica_flutter/editor/markdown.dart' show markdownToBlocks;
import 'package:mica_flutter/main.dart' as app;

import 'inputs.g.dart';

@JS('micaMdReady')
external JSPromise<JSAny?> get _ready;
@JS('micaMd')
external _MicaMd get _md;
@JS('performance.now')
external double _now();

/// Prototype of the web codec in architecture §5 (02 §4.4).
extension type _MicaMd._(JSObject _) implements JSObject {
  external String parseBody(String markdown, bool fragment);
  external double parseOnlyDigest(String markdown);
  external double parseIdDigest(String markdown, bool uuid);
  external String echo(String payload);
  external void noop();
  external String engineInfo();
}

void main() {
  globalContext['runMdP0'] =
      (([JSAny? keys, JSAny? stages]) => _run(keys, stages).toJS).toJS;
  app.main();
}

const _inputs = ['p0_87', 'p0_10k', 'p0_100k', 's0_87', 's0_10k', 's0_100k'];

int _sink = 0;

/// Full production web path: wasm parseBody → JSON string → jsonDecode →
/// BlockSpec records.
int _full(String src) {
  final rows = jsonDecode(_md.parseBody(src, false)) as List<dynamic>;
  final blocks = _mapRows(rows);
  return blocks.length + (blocks.isEmpty ? 0 : blocks.last.text.length);
}

List<({String kind, String text, Map<String, dynamic> data})> _mapRows(
    List<dynamic> rows) {
  return rows
      .map((row) {
        final b = row as Map<String, dynamic>;
        return (
          kind: b['type'] as String,
          text: b['text'] as String,
          data: b['data'] as Map<String, dynamic>,
        );
      })
      .toList();
}

int _dart(String src) {
  final b = markdownToBlocks(src);
  return b.length + (b.isEmpty ? 0 : b.last.text.length);
}

double _batch(void Function() body, int n) {
  final start = _now();
  for (var i = 0; i < n; i++) {
    body();
  }
  return (_now() - start) / n;
}

int _calibrate(void Function() body) {
  var n = 1;
  while (n < 8192) {
    if (_batch(body, n) * n >= 15) return n;
    n *= 2;
  }
  return n;
}

Map<String, Object> _stats(
    double Function() body, int n, List<double> samples) {
  samples.sort();
  return {
    'medianMs': samples[samples.length ~/ 2],
    'p95Ms': samples[(samples.length * .95).floor()],
    'batchSize': n,
    'samples': samples,
  };
}

/// Canonical form for cross-implementation comparison: ids/children dropped,
/// marks sorted by (start, end, type). Key order inside `data` is normalised by
/// sorting keys, so only values matter.
String _canonical(List<dynamic> rows) => jsonEncode(_canonicalBlocks(rows));

List<String> _canonicalBlocks(List<dynamic> rows) {
  final out = <String>[];
  for (final row in rows) {
    final b = row as Map<String, dynamic>;
    final data = <String, Object?>{};
    for (final key in (b['data'] as Map<String, dynamic>).keys.toList()..sort()) {
      final value = (b['data'] as Map<String, dynamic>)[key];
      if (key == 'marks' && value is List) {
        final marks = value.map((m) {
          // JSON object key order is not semantic; normalise mark maps too, or
          // every comparison reports a false diff on {"start","end"} order.
          final raw = m as Map;
          final norm = <String, Object?>{};
          for (final k in raw.keys.map((e) => e.toString()).toList()..sort()) {
            norm[k] = raw[k];
          }
          return norm;
        }).toList()
          ..sort((a, b) {
            final byStart = (a['start'] as int).compareTo(b['start'] as int);
            if (byStart != 0) return byStart;
            final byEnd = (a['end'] as int).compareTo(b['end'] as int);
            if (byEnd != 0) return byEnd;
            return (a['type'] as String).compareTo(b['type'] as String);
          });
        data[key] = marks;
      } else {
        data[key] = value;
      }
    }
    out.add(jsonEncode({'type': b['type'], 'text': b['text'], 'data': data}));
  }
  return out;
}

int _fnv(String s) {
  var h = 0x811c9dc5;
  for (final unit in s.codeUnits) {
    h = (h ^ unit) & 0xffffffff;
    h = (h * 0x01000193) & 0xffffffff;
  }
  return h;
}

Future<JSString> _run(JSAny? keysArg, JSAny? stagesArg) async {
  await _ready.toDart.timeout(const Duration(seconds: 10));
  final requested = keysArg == null ? _inputs : (keysArg.dartify() as List).cast<String>();
  final wanted = stagesArg == null
      ? null
      : (stagesArg.dartify() as List).cast<String>().toSet();
  bool uuidAvailable = true;
  try {
    _md.parseIdDigest(benchInputs['p0_87']!, true);
  } catch (_) {
    uuidAvailable = false;
  }

  final result = <String, Object>{'uuidAvailable': uuidAvailable};
  final perInput = <String, Object>{};

  for (final key in requested) {
    final src = benchInputs[key]!;
    final json = _md.parseBody(src, false); // pre-generated, outside samples
    final decoded = jsonDecode(json) as List<dynamic>;
    final rustBlocks = _canonicalBlocks(decoded);
    final dartBlocks = _canonicalBlocks(markdownToBlocks(src).map((b) => {
          'type': b.kind,
          'text': b.text,
          'data': b.data,
        }).toList());
    final canonicalRust = jsonEncode(rustBlocks);
    final canonicalDart = jsonEncode(dartBlocks);
    var differingBlocks = 0;
    for (var i = 0; i < math.min(rustBlocks.length, dartBlocks.length); i++) {
      if (rustBlocks[i] != dartBlocks[i]) differingBlocks++;
    }
    final blockCountDelta = (rustBlocks.length - dartBlocks.length).abs();

    final stages = <String, Map<String, Object>>{};
    void add(String name, void Function() body) {
      if (wanted != null && !wanted.contains(name)) return;
      final n = _calibrate(body);
      for (var i = 0; i < 20; i++) {
        body();
      }
      final samples = <double>[];
      for (var i = 0; i < 15; i++) {
        samples.add(_batch(body, n));
        _sink += i;
      }
      stages[name] = _stats(() => 0, n, samples);
    }

    add('loopOverhead', () => _sink++);
    add('wasmNoop', () => _md.noop());
    add('echoMd', () => _sink += _md.echo(src).length);
    add('echoJson', () => _sink += _md.echo(json).length);
    // Paired comparisons use AB/BA sample order, not two consecutive long
    // measurement windows. Keep sample order alongside sorted statistics.
    void pair(String a, void Function() fa, String b, void Function() fb) {
      if (wanted != null && !wanted.contains(a) && !wanted.contains(b)) return;
      for (var i=0; i<20; i++) { fa(); fb(); }
      final na=_calibrate(fa), nb=_calibrate(fb);
      final sa=<double>[], sb=<double>[];
      for (var i=0; i<15; i++) {
        if (i.isEven) {
          sa.add(_batch(fa,na)); sb.add(_batch(fb,nb));
        } else {
          sb.add(_batch(fb,nb)); sa.add(_batch(fa,na));
        }
      }
      final rawA=List<double>.of(sa), rawB=List<double>.of(sb);
      stages[a]={..._stats(()=>0,na,sa),'orderedSamples':rawA};
      stages[b]={..._stats(()=>0,nb,sb),'orderedSamples':rawB};
    }
    add('parseOnly', () => _sink += _md.parseOnlyDigest(src).toInt());
    if (uuidAvailable) {
      pair('sequentialParse', () => _sink += _md.parseIdDigest(src,false).toInt(),
        'uuidParse', () => _sink += _md.parseIdDigest(src,true).toInt());
    }
    add('parseBody', () => _sink += _md.parseBody(src, false).length);
    add('dartDecode', () => _sink += (jsonDecode(json) as List).length);
    add('dartMap', () => _sink += _mapRows(decoded).length);
    pair('full', () => _sink += _full(src), 'dartMarkdown', () => _sink += _dart(src));

    final full = (stages['full']?['medianMs'] as double?) ?? 0;
    final dart = (stages['dartMarkdown']?['medianMs'] as double?) ?? 1;
    perInput[key] = {
      'characters': src.length,
      'jsonBytes': json.length,
      'blocks': decoded.length,
      'stages': stages,
      'ratioFullVsDart': full / dart,
      'equivalence': {
        'equal': canonicalRust == canonicalDart,
        'differingBlocks': differingBlocks,
        'blockCountDelta': blockCountDelta,
        'rustFnv': _fnv(canonicalRust),
        'dartFnv': _fnv(canonicalDart),
        'firstDiff': _firstDiff(canonicalRust, canonicalDart),
        'rustLength': canonicalRust.length,
        'dartLength': canonicalDart.length,
      },
    };
  }
  result['inputs'] = perInput;
  result['engineInfo'] = _md.engineInfo();
  result['sink'] = _sink;
  return jsonEncode(result).toJS;
}

String? _firstDiff(String a, String b) {
  if (a == b) return null;
  final limit = math.min(a.length, b.length);
  var i = 0;
  while (i < limit && a.codeUnitAt(i) == b.codeUnitAt(i)) {
    i++;
  }
  final from = math.max(0, i - 80);
  final to = math.min(limit, i + 160);
  return jsonEncode({
    'at': i,
    'rust': a.substring(from, to),
    'dart': b.substring(from, math.min(b.length, from + (to - from))),
  });
}
