// Link-parity cases the shared conformance fixtures can't cover, because these
// inputs are not round-trip-stable (the fixture suite also asserts
// import->export->import stability, which `[](/url)` and a def whose label
// contains a link both break).
//
// Every expectation here is the RUST engine's actual output, captured from
// `mica_markdown::import_markdown` — not a guess. Rust is the authority; this
// pins the Dart mirror to it.
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/markdown.dart';
import 'package:mica_flutter/editor/marks.dart';

/// `type:start-end:href` triples, sorted — mark array order is semantically
/// irrelevant and the two engines emit it differently.
List<String> marksOf(Map<String, dynamic> data) {
  final raw = data['marks'];
  if (raw is! List) return const [];
  final out = raw
      .map((m) =>
          '${(m as Map)['type']}:${m['start']}-${m['end']}:${m['href'] ?? ''}')
      .toList()
    ..sort();
  return out;
}

void main() {
  test('empty link label is a zero-width anchor, like Rust', () {
    // Rust: text "x  y", marks [link 2-2 /url]. Dart used to leave this literal
    // (its `close > i + 1` demanded a non-empty label) AND then autolink the
    // bare URL — different text AND different marks from the server.
    final blocks = markdownToBlocks('x [](/url) y');
    expect(blocks.length, 1);
    expect(blocks.first.text, 'x  y');
    expect(marksOf(blocks.first.data), ['link:2-2:/url']);
  });

  test('reference form: a link-containing label disqualifies the outer link', () {
    // Rust: text "[foo bar]ref" — the outer reference is refused (its label
    // holds a link), the inner link stays, and `[ref]` then resolves on its own
    // as a shortcut reference.
    final blocks = markdownToBlocks('[foo [bar](/u)][ref]\n\n[ref]: /r');
    expect(blocks.length, 1);
    expect(blocks.first.text, '[foo bar]ref');
    expect(marksOf(blocks.first.data), ['link:5-8:/u', 'link:9-12:/r']);
  });

  test('an IMAGE label does not disqualify the outer link', () {
    // `[![alt](img)](url)` is a valid linked image — only `link` marks in the
    // label disqualify, never images. Asserted at the INLINE layer, which is
    // where the nested-link guard lives and where Rust agrees exactly
    // (Rust inline: text "a", image 0-1 /img + link 0-1 /u).
    //
    // NOT asserted through markdownToBlocks: Dart's BLOCK layer promotes a
    // paragraph holding only an image into an `image` block and drops the
    // surrounding link, where Rust keeps a paragraph carrying both marks. That
    // is a separate, pre-existing block-layer difference — see
    // docs/code-review-2026-07-20.md — and asserting it here would conflate the
    // two layers.
    final p = parseInline('[![a](/img)](/u)');
    expect(p.text, 'a');
    final got = p.marks
        .map((m) => '${m.type}:${m.start}-${m.end}:${m.href ?? ''}')
        .toList()
      ..sort();
    expect(got, ['image:0-1:/img', 'link:0-1:/u']);
  });
}
