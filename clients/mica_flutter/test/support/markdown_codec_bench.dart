import 'dart:convert';

import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'markdown_bench_inputs.dart';

/// Exact-input codec benchmark, codec-only since S3 T3.6.
///
/// Until T3.6 every row was a RATIO against the Dart mirror (codec ≤ 1.2×
/// mirror), which was the P0/M2 budget shape. The mirror is deleted, so the
/// ratio has no denominator; the rows are now absolute ceilings with generous
/// headroom over the measured VM-JIT numbers (parse.87 0.024 ms, parse.10k
/// 4.7 ms, parse.100k 52 ms, serialize.10k 2.7 ms, html.10k 3.1 ms — see
/// build/wp-d-vm-codec-bench.json before this change). They catch a gross
/// regression, not a few percent: pinpointing ratios again would need a second
/// implementation, which is exactly what S3 removed.
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
  void row(String item, double codecMs, double budgetMs) {
    final pass = codecMs <= budgetMs;
    rows.add({
      'item': item,
      'codec_ms': codecMs,
      'budget_ms': budgetMs,
      'passes': pass,
    });
    // ignore: avoid_print
    print(jsonEncode(rows.last));
  }

  for (final (label, text, batch, budget) in [
    ('87', hotPath87, 256, 0.1),
    ('10k', repeatTo(10000), 8, 15.0),
    ('100k', repeatTo(100000), 2, 150.0),
  ]) {
    final ms = median(() {
      final result = codec.parseBlocks(text);
      checksum ^=
          result.length + (result.isEmpty ? 0 : result.first.text.length);
    }, batch);
    row('parse.$label', ms, budget);
  }
  final blocks = codec.parseBlocks(repeatTo(10000));
  row(
    'serialize.10k',
    median(() {
      checksum ^= codec.serializeBlocks(blocks).length;
    }, 8),
    10.0,
  );
  row(
    'html.10k',
    median(() {
      checksum ^= codec.renderClipboardHtml(blocks).length;
    }, 8),
    10.0,
  );
  // 02 §7 gates a copy by its COMBINED time: Markdown + Clipboard HTML of the
  // same 10k selection, ≤ 16 ms (one frame). The two rows above are its parts.
  row(
    'copy.10k',
    median(() {
      checksum ^= codec.serializeBlocks(blocks).length;
      checksum ^= codec.renderClipboardHtml(blocks).length;
    }, 8),
    16.0,
  );
  row(
    'boundary.inline87',
    median(() {
      checksum ^= codec.parseInline(hotPath87).text.length;
    }, 256),
    0.1,
  );
  // ignore: avoid_print
  print(jsonEncode({'consumed_checksum': checksum}));
  return rows;
}