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
    // Not autolinks:
    expect(parseInline('<not a url>').marks, isEmpty);
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
}
