import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/render.dart';

/// Table cells store raw inline-Markdown source; painting renders it
/// (Typora-style) while the overlay editor keeps showing the source.
void main() {
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
    final span = RenderDocument.cellDisplaySpan(r'run `kubectl get pods` now', base);
    expect(span.toPlainText(), 'run kubectl get pods now');
    final code = leaves(span).firstWhere((s) => s.text == 'kubectl get pods');
    expect(code.style?.fontFamily, isNotNull,
        reason: 'code run must carry the monospace code style');
    expect(code.style?.fontFamily == base.fontFamily, isFalse);
  });

  test('bold renders heavy, markers stripped', () {
    final span = RenderDocument.cellDisplaySpan(r'list **pods** fast', base);
    expect(span.toPlainText(), 'list pods fast');
    final bold = leaves(span).firstWhere((s) => s.text == 'pods');
    expect(bold.style?.fontWeight, FontWeight.w700);
  });

  test('plain text and unmatched markers stay literal', () {
    expect(RenderDocument.cellDisplaySpan('plain', base).toPlainText(), 'plain');
    expect(
      RenderDocument.cellDisplaySpan('a ** b ` c', base).toPlainText(),
      'a ** b ` c',
      reason: 'unmatched markers degrade gracefully to literal text',
    );
  });

  test('empty cell keeps its space placeholder', () {
    expect(RenderDocument.cellDisplaySpan('', base).toPlainText(), ' ');
  });
}
