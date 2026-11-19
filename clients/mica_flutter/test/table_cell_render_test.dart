import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show ValueNotifier;
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/inline_code_layout.dart';
import 'package:mica_flutter/editor/model.dart' show EditorNode, kMonoFont;
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/editor/table.dart' show TableCell, TableData;
import 'package:mica_flutter/markdown/markdown_codec.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';
import 'support/markdown_codec.dart';

class _PillRecordingCanvas implements Canvas {
  _PillRecordingCanvas(this._inner);
  final Canvas _inner;
  final List<({RRect rrect, Color color})> rrects = [];

  @override
  void drawRRect(RRect rrect, Paint paint) {
    rrects.add((rrect: rrect, color: paint.color));
    _inner.drawRRect(rrect, paint);
  }

  @override
  dynamic noSuchMethod(Invocation invocation) {
    return Function.apply(
      // Forward every other Canvas call to the real recorder canvas.
      _forward(invocation.memberName),
      invocation.positionalArguments,
      invocation.namedArguments,
    );
  }

  Function _forward(Symbol name) => switch (name) {
    #save => _inner.save,
    #saveLayer => _inner.saveLayer,
    #restore => _inner.restore,
    #restoreToCount => _inner.restoreToCount,
    #getSaveCount => _inner.getSaveCount,
    #translate => _inner.translate,
    #scale => _inner.scale,
    #rotate => _inner.rotate,
    #skew => _inner.skew,
    #transform => _inner.transform,
    #getTransform => _inner.getTransform,
    #clipRect => _inner.clipRect,
    #clipRRect => _inner.clipRRect,
    #clipPath => _inner.clipPath,
    #getLocalClipBounds => _inner.getLocalClipBounds,
    #getDestinationClipBounds => _inner.getDestinationClipBounds,
    #drawColor => _inner.drawColor,
    #drawLine => _inner.drawLine,
    #drawPaint => _inner.drawPaint,
    #drawRect => _inner.drawRect,
    #drawDRRect => _inner.drawDRRect,
    #drawOval => _inner.drawOval,
    #drawCircle => _inner.drawCircle,
    #drawArc => _inner.drawArc,
    #drawPath => _inner.drawPath,
    #drawImage => _inner.drawImage,
    #drawImageRect => _inner.drawImageRect,
    #drawImageNine => _inner.drawImageNine,
    #drawPicture => _inner.drawPicture,
    #drawParagraph => _inner.drawParagraph,
    #drawPoints => _inner.drawPoints,
    #drawRawPoints => _inner.drawRawPoints,
    #drawVertices => _inner.drawVertices,
    #drawAtlas => _inner.drawAtlas,
    #drawRawAtlas => _inner.drawRawAtlas,
    #drawShadow => _inner.drawShadow,
    _ => throw UnsupportedError('Unhandled Canvas method: $name'),
  };
}

class _FakePaintingContext extends PaintingContext {
  _FakePaintingContext(this._canvas)
    : super(ContainerLayer(), const Rect.fromLTWH(0, 0, 1000, 1000));
  final Canvas _canvas;

  @override
  Canvas get canvas => _canvas;
}

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

  test('gh#188: table cells render inline-code pills, mono font, and spacer gaps', () {
    const md = '''
| 认证方式       | 需要填写的内容                                               | 凭证从哪里获取                              |
| -------------- | ------------------------------------------------------------ | ------------------------------------------- |
| Token Auth     | `INFISICAL_URL`、`INFISICAL_TOKEN`                           | `greenstor-ai-admin` 的 Token Auth 页面     |
| Universal Auth | `INFISICAL_URL`、`INFISICAL_CLIENT_ID`、`INFISICAL_CLIENT_SECRET` | `greenstor-ai-admin` 的 Universal Auth 页面 |
''';
    final blocks = markdownCodec.parseBlocks(md);
    final tableSpec = blocks.singleWhere((b) => b.kind == 'table');
    final table = TableData.fromBlock(tableSpec.data);

    // 1. Model keeps exact inline-code marks without misparsing underscores.
    final r1c1 = table.rows[1][1];
    expect(r1c1.text, 'INFISICAL_URL、INFISICAL_TOKEN');
    expect(r1c1.marks.map((m) => m.type).toList(), ['code', 'code']);

    final r2c1 = table.rows[2][1];
    expect(r2c1.text, 'INFISICAL_URL、INFISICAL_CLIENT_ID、INFISICAL_CLIENT_SECRET');
    expect(r2c1.marks.map((m) => m.type).toList(), ['code', 'code', 'code']);

    // 2. Display span inserts spacers and applies kMonoFont + kInlineCodeFontScale.
    final display = buildTableCellDisplay(r1c1, base, MicaTokens.light);
    expect(display.codeRanges, hasLength(2));
    expect(display.dims, hasLength(4));
    final codeLeaves = leaves(display.span)
        .where((s) => s.text == 'INFISICAL_URL' || s.text == 'INFISICAL_TOKEN')
        .toList();
    expect(codeLeaves, hasLength(2));
    for (final leaf in codeLeaves) {
      expect(leaf.style?.fontFamily, kMonoFont);
      expect(leaf.style?.color, MicaTokens.light.editor.inlineCodeInk);
      expect(leaf.style?.fontSize, closeTo(base.fontSize! * kInlineCodeFontScale, 1e-6));
    }

    // 3. Real RenderDocument paint draws inline-code background pills for every
    //    inline-code span in the table (7 runs total across rows 1-2).
    for (final tokens in [MicaTokens.light, MicaTokens.dark_]) {
      final blink = ValueNotifier<bool>(false);
      final render = RenderDocument(
        nodes: [EditorNode(id: 't1', kind: 'table', text: '', data: table.toBlockData())],
        appearance: EditorAppearance(tokens: tokens),
        selection: null,
        showCaret: false,
        caretBlink: blink,
      );
      render.layout(const BoxConstraints(maxWidth: 960), parentUsesSize: true);
      final recorder = ui.PictureRecorder();
      final spy = _PillRecordingCanvas(
        Canvas(recorder, const Rect.fromLTWH(0, 0, 1000, 1000)),
      );
      render.paint(_FakePaintingContext(spy), Offset.zero);
      final pic = recorder.endRecording();
      pic.dispose();
      render.dispose();
      blink.dispose();

      final pillColor = tokens.editor.inlineCodeBg;
      final pills = spy.rrects
          .where((r) => r.color.toARGB32() == pillColor.toARGB32())
          .toList();
      expect(
        pills.length,
        greaterThanOrEqualTo(7),
        reason: 'every inline-code run in the table must paint a rounded pill',
      );
    }
  });
}
