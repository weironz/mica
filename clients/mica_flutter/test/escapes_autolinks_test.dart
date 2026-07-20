import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/marks.dart';

void main() {
  test('backslash escapes neutralize inline markers', () {
    final p = parseInline(r'\*not bold\* and snake\_case');
    expect(p.text, '*not bold* and snake_case');
    expect(p.marks, isEmpty);
  });

  test('escaped delimiter is literal; spec pairing applies around it', () {
    // cmark: <p><em><em>bold *</em> still</em>*</p>
    final p = parseInline(r'**bold \** still**');
    expect(p.text, 'bold * still*');
    expect(p.marks.where((m) => m.type == 'italic').length, 2);
    expect(p.marks.where((m) => m.type == 'bold'), isEmpty);
  });

  test('autolinks: absolute URI and email', () {
    final p = parseInline('see <https://e.com/a?b=1> or <u@e.com>');
    expect(p.text, 'see https://e.com/a?b=1 or u@e.com');
    expect(p.marks[0].href, 'https://e.com/a?b=1');
    expect(p.marks[1].href, 'mailto:u@e.com');
    // Not an autolink — but a valid raw-HTML tag shape (spec behavior):
    final h = parseInline('<not a url>');
    expect(h.text, '<not a url>');
    expect(h.marks.single.type, 'html');
    // Not html either — stays literal text with no marks:
    expect(parseInline('<no closer').marks, isEmpty);
  });

  test('nested marks render properly nested (old segment bug)', () {
    final p = parseInline('**bold with *inner italic* kept**');
    final md = inlineToMarkdown(p.text, p.marks);
    expect(md, '**bold with *inner italic* kept**');
  });

  test('render escapes literals, keeps code raw, emits autolinks', () {
    final p = parseInline(r'`a *b*` then <https://e.co> and *i*');
    final md = inlineToMarkdown(p.text, p.marks);
    expect(md, '`a *b*` then <https://e.co> and *i*');
    // Literal stars in marked text get escaped on the way out.
    final q = parseInline('**a * b**');
    expect(inlineToMarkdown(q.text, q.marks), r'**a \* b**');
  });

  test('block leaders escape so paragraphs stay paragraphs', () {
    expect(escapeBlockLeader('- not a list'), r'\- not a list');
    expect(escapeBlockLeader('1. not numbered'), r'1\. not numbered');
    expect(escapeBlockLeader('# not heading'), r'\# not heading');
    expect(escapeBlockLeader('---'), r'\---');
    expect(escapeBlockLeader('plain'), 'plain');
  });

  // P1-1: setext underlines + spaced dividers used to slip through the Dart
  // mirror and, on re-import, promoted the PREVIOUS paragraph to a heading.
  // These must match crates/markdown/src/lib.rs escape_block_leader.
  test('setext underlines and spaced dividers escape too', () {
    expect(escapeBlockLeader('==='), r'\===');
    expect(escapeBlockLeader('=='), r'\==');
    expect(escapeBlockLeader('--'), r'\--'); // setext H2 underline
    expect(escapeBlockLeader('-- -'), r'\-- -'); // thematic break (spaces stripped)
    expect(escapeBlockLeader('####### x'), r'\####### x'); // any # count, like Rust
    // Non-underline lines with `=` stay untouched.
    expect(escapeBlockLeader('a = b'), 'a = b');
    expect(escapeBlockLeader('=x='), '=x=');
  });
}
