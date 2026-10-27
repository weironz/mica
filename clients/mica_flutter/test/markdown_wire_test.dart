import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/markdown/mark.dart';
import 'package:mica_flutter/markdown/markdown_wire.dart';

void main() {
  test('wire accepts serde-defaulted block and inline fields', () {
    final block = decodeBlocks('[{"type":"paragraph","data":null}]').single;
    expect(block.text, '');
    expect(block.data, isEmpty);
    expect(decodeInline({'text': 'plain'}).marks, isEmpty);
  });
  test('wire preserves block data and UTF-16 marks', () {
    final blocks = decodeBlocks(
      '[{"type":"paragraph","text":"😀x","data":{"marks":[{"start":2,"end":3,"type":"bold"}]}}]',
    );
    expect(blocks.single.kind, 'paragraph');
    expect(
      jsonDecode(encodeBlocks(blocks)),
      jsonDecode(
        '[{"type":"paragraph","text":"😀x","data":{"marks":[{"start":2,"end":3,"type":"bold"}]}}]',
      ),
    );
    final inline = (
      text: '😀x',
      marks: [Mark(2, 3, 'link', href: '/x', title: 'title')],
    );
    final decoded = decodeInline(encodeInline(inline));
    expect(decoded.text, inline.text);
    expect(decoded.marks.single.toJson(), inline.marks.single.toJson());
  });
}
