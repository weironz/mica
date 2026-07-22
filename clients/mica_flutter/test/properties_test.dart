// Test data mixes const values with the non-const text()/list() helpers, so the
// const-constructor lint is just noise here.
// ignore_for_file: prefer_const_constructors
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/properties.dart';

// Mirror of the Rust authority's unit tests (crates/markdown/src/properties.rs).
// Both suites assert the SAME behaviour — that is the whole point of the mirror.

PropertyValue text(String s) => PropText(s);
PropertyValue list(List<String> items) => PropList(items);

void main() {
  test('parses the flat typed subset', () {
    const fm = 'title: My Page\n'
        'tags: [work, urgent]\n'
        'count: 3\n'
        'ratio: 1.5\n'
        'done: true\n'
        'due: 2026-07-22\n'
        'authors:\n  - Alice\n  - Bob';
    expect(parseProperties(fm), [
      Property('title', text('My Page')),
      Property('tags', list(['work', 'urgent'])),
      Property('count', const PropNumber(3.0)),
      Property('ratio', const PropNumber(1.5)),
      Property('done', const PropCheckbox(true)),
      Property('due', const PropDate('2026-07-22')),
      Property('authors', list(['Alice', 'Bob'])),
    ]);
  });

  test('comments, blanks and unknown structure are skipped, not surfaced', () {
    const fm = '# a comment\n'
        'title: Hi\n'
        '\n'
        'nested:\n  child: 1\n'
        'after: ok';
    expect(parseProperties(fm), [
      Property('title', text('Hi')),
      Property('after', text('ok')),
    ]);
  });

  test('ambiguous scalars stay text to keep parse-render stable', () {
    expect(parseProperties('a: 007'), [Property('a', text('007'))]);
    expect(parseProperties('a: 1.0'), [Property('a', text('1.0'))]);
    expect(parseProperties('a: "3"'), [Property('a', text('3'))]);
  });

  test('upsert edits only the target key, leaving others byte-exact', () {
    const fm = '# keep me\n'
        'title: Old  # trailing comment kept\n'
        'tags: [a, b]\n'
        "note: 'single quoted'";
    final out = upsertProperty(fm, 'tags', list(['a', 'b', 'c']));
    expect(
      out,
      '# keep me\n'
      'title: Old  # trailing comment kept\n'
      'tags: [a, b, c]\n'
      "note: 'single quoted'",
    );
  });

  test('upsert preserves block-list style', () {
    const fm = 'authors:\n  - Alice\ntitle: X';
    expect(
      upsertProperty(fm, 'authors', list(['Alice', 'Bob'])),
      'authors:\n  - Alice\n  - Bob\ntitle: X',
    );
  });

  test('upsert appends a new key', () {
    expect(upsertProperty('title: X', 'done', const PropCheckbox(true)),
        'title: X\ndone: true');
    expect(upsertProperty('', 'title', text('First')), 'title: First');
    expect(upsertProperty('a: 1\n', 'b', const PropNumber(2.0)), 'a: 1\nb: 2');
  });

  test('remove deletes the key and its block, leaving the rest', () {
    const fm = 'title: X\nauthors:\n  - Alice\n  - Bob\ndone: true';
    expect(removeProperty(fm, 'authors'), 'title: X\ndone: true');
    expect(removeProperty(fm, 'missing'), fm);
  });

  test('parse then render is stable for every type', () {
    final cases = <Property>[
      Property('s', text('hello world')),
      Property('s', text('')),
      Property('s', text('needs: quoting')),
      Property('s', text('true')),
      Property('s', text('42')),
      Property('n', const PropNumber(3.0)),
      Property('n', const PropNumber(-2.5)),
      Property('b', const PropCheckbox(false)),
      Property('d', const PropDate('2026-01-09')),
      Property('l', list(['x', 'y, z', 'true'])),
      Property('l', list([])),
    ];
    for (final c in cases) {
      // Render via upsert into an empty document, then re-parse.
      final rendered = upsertProperty('', c.key, c.value);
      expect(parseProperties(rendered), [c],
          reason: 'parse∘render unstable for ${c.value} -> $rendered');
    }
  });

  test('tags are just a list-valued property', () {
    expect(parseProperties('tags: [rust, crdt]'),
        [Property('tags', list(['rust', 'crdt']))]);
  });

  test('json round-trips a value across the FFI shape', () {
    for (final v in <PropertyValue>[
      text('hi'),
      const PropNumber(4.0),
      const PropCheckbox(true),
      const PropDate('2026-07-22'),
      list(['a', 'b']),
    ]) {
      expect(PropertyValue.fromJson(v.toJson()), v);
    }
  });
}
