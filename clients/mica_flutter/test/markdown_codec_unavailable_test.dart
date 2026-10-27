import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';

void main() {
  test('unavailable codec rejects every operation', () {
    const codec = UnavailableMarkdownCodec();
    final calls = <void Function()>[
      () => codec.parseBlocks(''),
      () => codec.parseInline(''),
      () => codec.parseInlineBatch([]),
      () => codec.decodeCells([]),
      () => codec.encodeCells([]),
      () => codec.serializeBlocks([]),
      () => codec.serializeInline('', []),
      () => codec.renderClipboardHtml([]),
      () => codec.mathRunSpans(''),
      () => codec.parseInlineMath(''),
      () => codec.engineInfo,
    ];
    for (final call in calls) {
      expect(call, throwsA(isA<MarkdownCodecException>()));
    }
  });
  test('uninitialized accessor fails explicitly', () {
    debugSetMarkdownCodec(null);
    expect(() => markdownCodec, throwsStateError);
  });
}
