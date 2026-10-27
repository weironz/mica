import 'dart:convert';

import 'mark.dart';
import 'markdown_codec.dart';

List<BlockSpec> decodeBlocks(String json) =>
    (jsonDecode(json) as List).map((item) {
      final map = item as Map<String, dynamic>;
      return (
        kind: map['type'] as String,
        text: map['text'] as String? ?? '',
        data: Map<String, dynamic>.from(map['data'] as Map? ?? const {}),
      );
    }).toList();

String encodeBlocks(List<BlockSpec> blocks) => jsonEncode([
  for (final block in blocks)
    {'type': block.kind, 'text': block.text, 'data': block.data},
]);

InlineText decodeInline(Map<String, dynamic> map) => (
  text: map['text'] as String,
  marks: (map['marks'] as List? ?? const []).map((item) {
    final m = item as Map<String, dynamic>;
    return Mark(
      m['start'] as int,
      m['end'] as int,
      m['type'] as String,
      href: m['href'] as String?,
      title: m['title'] as String?,
    );
  }).toList(),
);

Map<String, dynamic> encodeInline(InlineText inline) => {
  'text': inline.text,
  'marks': [for (final mark in inline.marks) mark.toJson()],
};
List<InlineText> decodeInlines(String json) => (jsonDecode(json) as List)
    .map((item) => decodeInline(item as Map<String, dynamic>))
    .toList();
String encodeInlines(List<InlineText> values) =>
    jsonEncode([for (final value in values) encodeInline(value)]);
List<({int start, int end})> decodeSpans(String json) =>
    (jsonDecode(json) as List)
        .map((item) => (start: item[0] as int, end: item[1] as int))
        .toList();
