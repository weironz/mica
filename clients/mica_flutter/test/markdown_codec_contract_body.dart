import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/markdown/markdown_wire.dart';

import 'support/markdown_gold_io.dart'
    if (dart.library.js_interop) 'support/markdown_gold_web.dart';

({
  String function,
  List<({String id, Map<String, dynamic> args, String output})> cases,
})
parseWireGold(String source) {
  final document = jsonDecode(source) as Map<String, dynamic>;
  return (
    function: document['function'] as String,
    cases: (document['cases'] as List).map((entry) {
      final item = entry as Map<String, dynamic>;
      return (
        id: item['id'] as String,
        args: Map<String, dynamic>.from(item['args'] as Map),
        output: item['output'] as String,
      );
    }).toList(),
  );
}

void _expectBlocks(List<BlockSpec> actual, String goldOutput) {
  // Compare the codec's blocks directly against the raw gold values. Going
  // through encodeBlocks() again would re-apply facade defaults and hide a
  // wire difference (a field the engine omitted vs. one it emitted).
  final expected = jsonDecode(goldOutput) as List;
  expect(actual.length, expected.length);
  for (var i = 0; i < expected.length; i++) {
    final e = expected[i] as Map<String, dynamic>;
    final a = actual[i];
    expect(a.kind, e['type'], reason: 'block $i kind');
    // `text` is serde-defaulted: an OMITTED key equals "" (not a bug), but an
    // explicit null is not a legal wire value for a plain String field.
    expect(
      a.text,
      e.containsKey('text') ? e['text'] as String : '',
      reason: 'block $i text',
    );
    // §3.2: `data` is always an object on the wire. Omission is the serde
    // default (null), which the wire normalizes to {}; explicit null in the
    // OUTPUT is not allowed — distinguish the two instead of folding null
    // into the default.
    final data = e.containsKey('data') ? e['data'] : const <String, dynamic>{};
    expect(
      data,
      isA<Map<String, dynamic>>(),
      reason: 'block $i data must be an object (02 §3.2)',
    );
    expect(a.data, data, reason: 'block $i data');
  }
}

void _expectInline(InlineText actual, Object? gold) {
  final e = gold as Map<String, dynamic>;
  // `text` is required (no serde default): a missing or null key must fail.
  expect(actual.text, e['text'], reason: 'inline text');
  // `marks` is serde-defaulted: omission equals [], explicit null is illegal.
  final expectedMarks = e.containsKey('marks') ? e['marks'] as List : const [];
  expect(
    actual.marks.length,
    expectedMarks.length,
    reason: 'inline mark count',
  );
  for (var i = 0; i < expectedMarks.length; i++) {
    final m = expectedMarks[i] as Map<String, dynamic>;
    final a = actual.marks[i];
    // start/end/type are required; href/title are `skip_serializing_if`
    // Options, where omitted and null are BOTH spec-equivalent to None.
    expect(a.start, m['start'], reason: 'mark $i start');
    expect(a.end, m['end'], reason: 'mark $i end');
    expect(a.type, m['type'], reason: 'mark $i type');
    expect(a.href, m['href'], reason: 'mark $i href');
    expect(a.title, m['title'], reason: 'mark $i title');
  }
}

void checkWireGoldCase(
  MarkdownCodec codec,
  String function,
  Map<String, dynamic> args,
  String output,
) {
  String arg(String name) => args[name] as String;
  Map<String, String> strings(String name) =>
      Map<String, String>.from(jsonDecode(arg(name)) as Map);
  switch (function.replaceAll('_json', '')) {
    case 'parse_body':
      _expectBlocks(
        codec.parseBlocks(arg('markdown'), fragment: args['fragment'] as bool),
        output,
      );
    case 'parse_inline_batch':
      final sources = (jsonDecode(arg('sources_json')) as List).cast<String>();
      final expected = jsonDecode(output) as List;
      final actual = codec.parseInlineBatch(sources);
      expect(actual.length, expected.length);
      for (var i = 0; i < expected.length; i++) {
        _expectInline(actual[i], expected[i]);
      }
    case 'decode_cells':
      final expected = jsonDecode(output) as List;
      final actual = codec.decodeCells(jsonDecode(arg('cells_json')) as List);
      expect(actual.length, expected.length);
      for (var i = 0; i < expected.length; i++) {
        _expectInline(actual[i], expected[i]);
      }
    case 'encode_cells':
      expect(
        codec.encodeCells(decodeInlines(arg('inlines_json'))),
        jsonDecode(output),
      );
    case 'serialize_body':
      expect(
        codec.serializeBlocks(
          decodeBlocks(arg('blocks_json')),
          assets: strings('assets_json'),
        ),
        output,
      );
    case 'serialize_inline':
      final value = decodeInline(
        jsonDecode(arg('inline_json')) as Map<String, dynamic>,
      );
      expect(codec.serializeInline(value.text, value.marks), output);
    case 'render_html_body':
      // The frozen Dart facade exposes the clipboard profile only. The export
      // profile is verified at the wire/binding layer elsewhere; cross-platform
      // render parity is NOT yet established (M2 render conflict unresolved).
      if (arg('profile') != 'clipboard') {
        throw ArgumentError(
          'Dart facade does not expose the export profile; wire/binding layer tests it separately',
        );
      }
      expect(
        codec.renderClipboardHtml(
          decodeBlocks(arg('blocks_json')),
          imageSrcs: strings('image_srcs_json'),
        ),
        output,
      );
    case 'math_run_spans':
      expect(
        codec
            .mathRunSpans(arg('source'))
            .map((span) => [span.start, span.end])
            .toList(),
        jsonDecode(output),
      );
    case 'parse_inline_math':
      _expectInline(codec.parseInlineMath(arg('source')), jsonDecode(output));
    case 'properties':
      expect(
        codec.properties(
          Map<String, Object?>.from(jsonDecode(arg('request_json')) as Map),
        ),
        jsonDecode(output),
      );
    default:
      fail('Unknown wire function: $function');
  }
}

const wireGoldFunctions = [
  'parse_body',
  'parse_inline_batch',
  'decode_cells',
  'encode_cells',
  'serialize_body',
  'serialize_inline',
  'render_html_body',
  'math_run_spans',
  'parse_inline_math',
  'properties',
];

void runCodecContract(Future<MarkdownCodec> Function() make) {
  late MarkdownCodec codec;
  setUpAll(() async => codec = await make());
  test('MarkdownCodec engine identity and single-inline wrapper', () {
    final info = codec.engineInfo;
    expect(info.schemaVersion, 1);
    expect(info.crateVersion, isNotEmpty);
    expect(info.sourceFingerprint, matches(RegExp(r'^[0-9a-f]{16}$')));
    final batch = codec.parseInlineBatch(['😀 **single**']).single;
    final single = codec.parseInline('😀 **single**');
    expect(single.text, batch.text);
    expect(
      single.marks.map((mark) => mark.toJson()).toList(),
      batch.marks.map((mark) => mark.toJson()).toList(),
    );
  });
  for (final name in wireGoldFunctions) {
    test('MarkdownCodec wire gold: $name', () async {
      final gold = parseWireGold(await readMarkdownGold(name));
      expect(gold.function.replaceAll('_json', ''), name);
      expect(
        gold.cases,
        isNotEmpty,
        reason: 'Empty gold cannot prove coverage',
      );
      var checked = 0;
      for (final item in gold.cases) {
        if (name == 'render_html_body' && item.args['profile'] == 'export') {
          // Not a Dart facade case and NOT counted as passed or skipped: the
          // facade has no export-profile API. Wire/binding coverage is tested
          // separately; cross-platform render parity is not yet established.
          continue;
        }
        try {
          checkWireGoldCase(codec, gold.function, item.args, item.output);
        } catch (error, stack) {
          Error.throwWithStackTrace(
            StateError('$name/${item.id}: $error'),
            stack,
          );
        }
        checked++;
      }
      expect(checked, greaterThan(0), reason: 'No facade cases in $name gold');
    });
  }
}
