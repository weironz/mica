import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/import/import_title.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';

import 'support/markdown_codec.dart';

BlockSpec b(String kind, String text) => (kind: kind, text: text, data: const {});

void main() {
  setUpAll(useNativeMarkdownCodec);

  test('through the engine: inline markers dropped, front matter skipped', () {
    expect(titleFromMarkdown('# **Bold** title\n\nbody', 'f'), 'Bold title');
    expect(titleFromMarkdown('---\ntags: [a]\n---\n\n## Real\n', 'f'), 'Real');
    expect(titleFromMarkdown('', 'f'), 'f');
  });

  test('first heading names the page', () {
    expect(titleFromBlocks([b('heading', 'Bold title'), b('paragraph', 'x')], 'f'), 'Bold title');
  });

  test('a paragraph before any heading wins, first line only, capped at 60', () {
    final long = 'a' * 80;
    expect(titleFromBlocks([b('paragraph', '$long\nsecond'), b('heading', 'H')], 'f'), 'a' * 60);
  });

  test('blocks without text are skipped; none at all gives the fallback', () {
    expect(titleFromBlocks([b('image', ''), b('heading', '  ')], 'f'), 'f');
    expect(titleFromBlocks([b('divider', ''), b('paragraph', ' hi ')], 'f'), 'hi');
  });

  test('no engine: the fallback, not a guess from raw lines', () {
    expect(titleFromMarkdown('# Title', 'Imported', codec: const UnavailableMarkdownCodec()), 'Imported');
  });
}
