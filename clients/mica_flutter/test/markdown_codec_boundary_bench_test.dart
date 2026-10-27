@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/markdown/markdown_wire.dart';
import 'package:mica_flutter/src/rust/api/markdown.dart' as ffi;
import 'support/markdown_codec.dart';
import 'support/markdown_bench_inputs.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  test('87-unit full boundary and JSON stages diagnostic', () {
    final wire = ffi.mdParseBody(markdown: hotPath87, fragment: false);
    final decoded = jsonDecode(wire) as List;
    double measure(void Function() action) {
      for (var i = 0; i < 3000; i++) {
        action();
      }
      final values = <double>[];
      for (var sample = 0; sample < 15; sample++) {
        final watch = Stopwatch()..start();
        for (var i = 0; i < 1024; i++) {
          action();
        }
        watch.stop();
        values.add(watch.elapsedMicroseconds / 1000 / 1024);
      }
      values.sort();
      return values[7];
    }

    final result = {
      'mode': 'flutter-test VM diagnostic; stages not additive',
      'wire_rust_parse_json_and_ffi_ms': measure(() {
        ffi.mdParseBody(markdown: hotPath87, fragment: false);
      }),
      'dart_json_decode_ms': measure(() {
        jsonDecode(wire);
      }),
      'dart_block_map_ms': measure(() {
        decoded.map((raw) {
          final map = raw as Map<String, dynamic>;
          return (
            kind: map['type'] as String,
            text: map['text'] as String? ?? '',
            data: Map<String, dynamic>.from(map['data'] as Map? ?? const {}),
          );
        }).toList();
      }),
      'dart_decode_and_map_ms': measure(() {
        decodeBlocks(wire);
      }),
      'full_codec_ms': measure(() {
        markdownCodec.parseBlocks(hotPath87);
      }),
    };
    File(
      'build/wp-d-boundary-bench.json',
    ).writeAsStringSync(const JsonEncoder.withIndent(' ').convert(result));
    // ignore: avoid_print
    print(jsonEncode(result));
    expect(markdownCodec.parseBlocks(hotPath87), isNotEmpty);
  });
}
