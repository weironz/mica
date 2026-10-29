import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart' show TableCell;
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';
import 'support/markdown_codec.dart';

void main() {
  setUpAll(useNativeMarkdownCodec);
  // A stored source cell decoded through the engine, as the editor opens it.
  TableCell cell(String source) {
    final d = markdownCodec.decodeCells([source]).single;
    return TableCell(d.text, d.marks);
  }

  const base = TextStyle(fontSize: 15, color: Color(0xFF111827));

  List<TextSpan> leaves(TextSpan span) {
    final out = <TextSpan>[];
    void walk(InlineSpan s) {
      if (s is TextSpan) {
        if (s.children == null || s.children!.isEmpty) {
          out.add(s);
        } else {
          s.children!.forEach(walk);
        }
      }
    }

    walk(span);
    return out;
  }

  test('inline code renders styled, backticks stripped', () {
    final span = RenderDocument.cellDisplaySpan(
      cell(r'run `kubectl get pods` now'),
      base,
      MicaTokens.light,
    );
    expect(span.toPlainText(), 'run kubectl get pods now');
    final code = leaves(span).firstWhere((s) => s.text == 'kubectl get pods');
    expect(
      code.style?.fontFamily,
      isNotNull,
      reason: 'code run must carry the monospace code style',
    );
    expect(code.style?.fontFamily == base.fontFamily, isFalse);
  });

  test('bold renders heavy, markers stripped', () {
    final span = RenderDocument.cellDisplaySpan(
      cell(r'list **pods** fast'),
      base,
      MicaTokens.light,
    );
    expect(span.toPlainText(), 'list pods fast');
    final bold = leaves(span).firstWhere((s) => s.text == 'pods');
    expect(bold.style?.fontWeight, FontWeight.w600);
  });

  test('plain text and unmatched markers stay literal', () {
    expect(
      RenderDocument.cellDisplaySpan(
        cell('plain'),
        base,
        MicaTokens.light,
      ).toPlainText(),
      'plain',
    );
    expect(
      RenderDocument.cellDisplaySpan(
        cell('a ** b ` c'),
        base,
        MicaTokens.light,
      ).toPlainText(),
      'a ** b ` c',
      reason: 'unmatched markers degrade gracefully to literal text',
    );
  });

  test('empty cell keeps its space placeholder', () {
    expect(
      RenderDocument.cellDisplaySpan(TableCell.empty, base, MicaTokens.light).toPlainText(),
      ' ',
    );
  });
}
