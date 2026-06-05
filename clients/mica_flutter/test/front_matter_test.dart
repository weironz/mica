import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';

void main() {
  test('front matter is stripped, never a visible block', () {
    final blocks = markdownToBlocks(
      '---\ntitle: Hello\ntags: [a, b]\n---\n# Heading\n\nBody.',
    );
    // Metadata is dropped on paste; only the real content survives.
    expect(blocks.map((b) => b.kind).toList(), ['heading', 'paragraph']);
    expect(blocks.first.text, 'Heading');
    // No paragraph leaked the YAML text.
    expect(
      blocks.any((b) => b.text.contains('title')),
      isFalse,
      reason: 'front matter must not surface as text',
    );
  });

  test('the fences do not become dividers or setext headings', () {
    final blocks = markdownToBlocks('---\nkey: value\n---\nbody');
    expect(blocks.map((b) => b.kind).toList(), ['paragraph']);
    expect(blocks.single.text, 'body');
  });

  test('dot close fence is recognized', () {
    final blocks = markdownToBlocks('---\nkey: value\n...\nbody');
    expect(blocks.map((b) => b.kind).toList(), ['paragraph']);
    expect(blocks.single.text, 'body');
  });

  test('empty front matter is stripped', () {
    final blocks = markdownToBlocks('---\n---\nbody');
    expect(blocks.map((b) => b.kind).toList(), ['paragraph']);
    expect(blocks.single.text, 'body');
  });

  test('first line not a fence parses normally (setext heading)', () {
    // `Title\n---` is a setext H2, not an un-opened front matter fence.
    final blocks = markdownToBlocks('Title\n---\n');
    expect(blocks.first.kind, 'heading');
    expect(blocks.first.data['level'], 2);
  });

  test('unterminated fence is treated as body', () {
    // Leading `---` with no close is not front matter: it stays a divider.
    final blocks = markdownToBlocks('---\ntitle: Hello\nbody line\n');
    expect(blocks.first.kind, 'divider');
  });

  test('leading --- that is not the first line is not front matter', () {
    final blocks = markdownToBlocks('# Heading\n\n---\n\nBody.');
    expect(blocks.map((b) => b.kind).toList(),
        ['heading', 'divider', 'paragraph']);
  });
}
