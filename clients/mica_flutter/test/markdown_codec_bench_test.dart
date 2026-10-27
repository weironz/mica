@TestOn('vm')
library;

import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'support/markdown_codec.dart';
import 'support/markdown_codec_bench.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  test('VM FFI exact S0 input performance budgets', () {
    final results = measureMarkdownCodec(markdownCodec);
    File('build/wp-d-vm-codec-bench.json').writeAsStringSync(
      const JsonEncoder.withIndent(
        ' ',
      ).convert({'mode': 'flutter-test VM', 'results': results}),
    );
    expect(
      results.where((row) => row['passes'] != true).toList(),
      isEmpty,
      reason: 'Full exact-input codec path must stay within the frozen budgets',
    );
  }, timeout: const Timeout(Duration(minutes: 3)));
}
