import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/model.dart';

/// Math paste routing: a pasted line that is entirely one *display* formula
/// ($$…$$, \[…\]) becomes a standalone math block; *inline* forms ($…$,
/// \(…\)) are woven into the text flow as inline-math marks instead of
/// jumping onto their own line.
void main() {
  group('pastedFormulaSource: display delimiters → math block source', () {
    const cases = {
      r'$$E = mc^2$$': r'E = mc^2',
      r'\[E = mc^2\]': r'E = mc^2',
      r'$$x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}$$':
          r'x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}',
    };
    cases.forEach((input, want) {
      test(input, () => expect(pastedFormulaSource(input), want));
    });
  });

  group('pastedFormulaSource: inline forms & prose are NOT display blocks', () {
    const cases = [
      r'$E = mc^2$', // inline single-$ — stays in the text flow
      r'\(E = mc^2\)', // inline paren form
      r'see $x$ in the text',
      r'price is $5 and $10',
      r'just plain text',
      r'$',
      r'$$',
      '',
    ];
    for (final input in cases) {
      test(input.isEmpty ? '<empty>' : input,
          () => expect(pastedFormulaSource(input), isNull));
    }
  });

  group('parseInlineMath: weaves \$…\$ / \\(…\\) into math marks', () {
    test(r'a lone $E = mc^2$ becomes one marked run', () {
      final r = parseInlineMath(r'$E = mc^2$');
      expect(r.text, r'E = mc^2');
      expect(r.marks, hasLength(1));
      expect(r.marks.single.type, 'math');
      expect(r.marks.single.start, 0);
      expect(r.marks.single.end, r.text.length);
    });

    test(r'mid-prose: see $x^2$ here', () {
      final r = parseInlineMath(r'see $x^2$ here');
      expect(r.text, 'see x^2 here');
      expect(r.marks, hasLength(1));
      expect(r.text.substring(r.marks.single.start, r.marks.single.end), 'x^2');
    });

    test(r'\(…\) paren form', () {
      final r = parseInlineMath(r'\(a+b\) and \(c\)');
      expect(r.text, 'a+b and c');
      expect(r.marks, hasLength(2));
    });

    test(r'two dollar formulas on one line', () {
      final r = parseInlineMath(r'$a$ and $b$');
      expect(r.text, 'a and b');
      expect(r.marks, hasLength(2));
    });

    test(r'bare dollars (prices) are NOT math', () {
      // Pandoc rules: opener must not be followed by whitespace, closer must
      // not be preceded by whitespace — "$5 and $10" has no valid closer.
      final r = parseInlineMath(r'price is $5 and $10');
      expect(r.marks, isEmpty);
      expect(r.text, r'price is $5 and $10');
    });

    test(r'a $$ display marker is left alone', () {
      final r = parseInlineMath(r'$$E=mc^2$$');
      expect(r.marks, isEmpty);
      expect(r.text, r'$$E=mc^2$$');
    });

    test('plain text passes through verbatim', () {
      final r = parseInlineMath('no formulas at all');
      expect(r.marks, isEmpty);
      expect(r.text, 'no formulas at all');
    });
  });

  group('insertInlineSpan: pasting inline math into the text flow', () {
    EditorController fresh(String text) {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
      c.load([EditorNode(id: 'a', kind: 'paragraph', text: text)]);
      return c;
    }

    test('weaves into the middle of a line, caret after the run', () {
      final c = fresh('before  after');
      c.selection = DocSelection.collapsed(const DocPosition(0, 7));
      final parsed = parseInlineMath(r'$E=mc^2$');
      c.insertInlineSpan(0, 7, 7, parsed.text, parsed.marks);

      expect(c.nodes[0].text, 'before E=mc^2 after');
      expect(c.nodes[0].kind, 'paragraph',
          reason: 'inline math must NOT turn the block into a math_block');
      final marks = marksFromData(c.nodes[0].data);
      expect(marks, hasLength(1));
      expect(marks.single.type, 'math');
      expect(
          c.nodes[0].text.substring(marks.single.start, marks.single.end),
          'E=mc^2');
      expect(c.selection!.focus.offset, 7 + 'E=mc^2'.length);
    });

    test('existing marks shift across the insertion', () {
      final c = fresh('bold tail');
      // "bold" carries a bold mark; insert math before "tail".
      c.nodes[0].data = {
        'marks': [
          {'start': 0, 'end': 4, 'type': 'bold'},
          {'start': 5, 'end': 9, 'type': 'italic'},
        ],
      };
      c.selection = DocSelection.collapsed(const DocPosition(0, 5));
      c.insertInlineSpan(0, 5, 5, 'x+y ', [Mark(0, 3, 'math')]);

      expect(c.nodes[0].text, 'bold x+y tail');
      final marks = marksFromData(c.nodes[0].data);
      final bold = marks.firstWhere((m) => m.type == 'bold');
      final italic = marks.firstWhere((m) => m.type == 'italic');
      final math = marks.firstWhere((m) => m.type == 'math');
      expect((bold.start, bold.end), (0, 4), reason: 'before insertion: unmoved');
      expect((italic.start, italic.end), (9, 13), reason: 'after: shifted by 4');
      expect(c.nodes[0].text.substring(math.start, math.end), 'x+y');
    });

    test('a ranged selection is replaced by the pasted run', () {
      final c = fresh('keep REPLACED keep');
      c.selection = const DocSelection(
        anchor: DocPosition(0, 5),
        focus: DocPosition(0, 13),
      );
      c.insertInlineSpan(0, 5, 13, 'a^2', [Mark(0, 3, 'math')]);
      expect(c.nodes[0].text, 'keep a^2 keep');
    });

    test('refuses atomic / code / table blocks', () {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
      c.load([EditorNode(id: 'm', kind: 'math_block', text: 'x')]);
      c.selection = DocSelection.collapsed(const DocPosition(0, 0));
      c.insertInlineSpan(0, 0, 0, 'y', [Mark(0, 1, 'math')]);
      expect(c.nodes[0].text, 'x', reason: 'atomic blocks must be untouched');
    });
  });
}
