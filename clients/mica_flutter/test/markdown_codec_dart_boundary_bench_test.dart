@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart' as mirror;
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/markdown/markdown_wire.dart';
import 'package:mica_flutter/src/rust/api/markdown.dart' as ffi;
import 'support/markdown_codec.dart';
import 'support/markdown_bench_inputs.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  test('split Dart JSON encode/decode cost in html and serialize paths', () {
    String repeatTo(int length) {
      final text = StringBuffer();
      while (text.length < length) {
        text.write(markdownFixtureSample);
      }
      return text.toString().substring(0, length);
    }

    final source = repeatTo(10000);
    final blocks = mirror.markdownToBlocks(source);
    double measure(void Function() action) {
      for (var i = 0; i < 50; i++) {
        action();
      }
      final values = <double>[];
      for (var sample = 0; sample < 9; sample++) {
        final watch = Stopwatch()..start();
        for (var i = 0; i < 20; i++) {
          action();
        }
        watch.stop();
        values.add(watch.elapsedMicroseconds / 1000 / 20);
      }
      values.sort();
      return values[4];
    }

    final result = <String, Object>{
      'mode': 'VM diagnostic: stage medians, not additive',
      'input_chars': source.length,
      'block_count': blocks.length,
      'encode_blocks_ms': measure(() {
        encodeBlocks(blocks);
      }),
      'html_ffi_only_ms': measure(() {
        ffi.mdRenderHtmlBody(
          blocksJson: encodeBlocks(blocks),
          imageSrcsJson: '{}',
          profile: 'clipboard',
        );
      }),
      'html_full_ms': measure(() {
        markdownCodec.renderClipboardHtml(blocks);
      }),
      'serialize_ffi_only_ms': measure(() {
        ffi.mdSerializeBody(blocksJson: encodeBlocks(blocks), assetsJson: '{}');
      }),
      'serialize_full_ms': measure(() {
        markdownCodec.serializeBlocks(blocks);
      }),
    };
    File(
      'build/wp-d-dart-boundary-split.json',
    ).writeAsStringSync(const JsonEncoder.withIndent(' ').convert(result));
    // ignore: avoid_print
    print(jsonEncode(result));
  });
}
