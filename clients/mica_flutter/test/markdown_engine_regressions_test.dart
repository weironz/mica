// Engine regression cases that still have a Dart-side subject after S3.
//
// The mirror-parser cases that used to live here went with the Dart mirror in
// S3 T3.9; Rust equivalents:
//   * Unicode indentation columns → crates/markdown/tests/unicode_indent.rs
//   * unclosed `$$` / `\[` keeps following lines →
//     crates/markdown/tests/unclosed_math.rs
//   * code-span whitespace payloads → crates/markdown/tests/code_whitespace.rs
//   * entities / math / strike / empty-link inline round trips →
//     crates/markdown/tests/inline_invertibility.rs
//   * quoted fences share top-level rules →
//     crates/markdown/tests/fence_round_trip.rs
//     `quoted_fences_share_top_level_delimiter_rules`
//   * zero-width link at a shorthand start →
//     crates/markdown/tests/inline_invertibility.rs
//     `zero_width_link_at_a_shorthand_start_survives_export`
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/properties.dart';
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  test('unmarked punctuation keeps the plain serializer contract', () {
    const text = '<]`\n;';
    expect(inlineToMarkdown(text, []), text);
    expect(inlineToMarkdown('&amp;', []), '&amp;amp;');
  });
  test('property lists quote every comma', () {
    for (final item in ['a,b', 'a, b', 'a,,b', ',a', 'a,']) {
      final value = PropList([item]);
      expect(parseProperties(upsertProperty('', 'tags', value)), [Property('tags', value)]);
    }
  });
  test('copied code block fences never conflict with content', () {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    addTearDown(c.dispose);
    const codeText = '````\n```\n````';
    c.load([
      EditorNode(id: 'b0', kind: 'code_block', text: codeText, data: {'language': 'rust'}),
    ]);
    c.selection = DocSelection(
      anchor: const DocPosition(0, 0),
      focus: DocPosition(0, codeText.length),
    );
    final md = c.selectionText();
    expect(md.startsWith('`````rust'), isTrue, reason: md);
    expect(md.endsWith('`````'), isTrue, reason: md);
    final again = markdownCodec.parseBlocks(md);
    expect(again.single.kind, 'code_block');
    expect(again.single.text, codeText);
  });
}