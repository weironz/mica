// Browser-only regression fixture. It renders the real editor and exposes only
// focus and rendering status; selection, copy and paste use real keyboard events.
import 'dart:js_interop';
import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/mermaid_preview.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';
import 'package:mica_flutter/ui/theme_tokens.dart';

@JS('micaClipboardHarnessFocus')
external set _focusEditor(JSFunction callback);

@JS('micaClipboardHarnessMathState')
external set _mathState(JSFunction callback);

@JS('micaPreviewHarnessHasHanGlyphs')
external set _hasHanGlyphs(JSFunction callback);

void main() {
  final isRenderFixture = Uri.base.queryParameters['fixture'] == 'render';
  final commands = EditorCommandHook();
  final editorKey = GlobalKey();
  runApp(
    MaterialApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(
        body: MicaEditor(
          key: editorKey,
          rootBlockId: 'root',
          nodes: isRenderFixture
              ? _renderFixture()
              : Uri.base.queryParameters['fixture'] == 'empty'
              ? [EditorNode(id: 'empty', kind: 'paragraph', text: '')]
              : Uri.base.queryParameters['fixture'] == 'inline'
              ? [
                  EditorNode(
                    id: 'inline',
                    kind: 'paragraph',
                    text: 'run command then `literal`',
                    data: {
                      'marks': marksToJson([Mark(4, 11, 'code')]),
                    },
                  ),
                  EditorNode(id: 'after', kind: 'paragraph', text: 'after'),
                ]
              : Uri.base.queryParameters['fixture'] == 'table'
              ? [
                  EditorNode(
                    id: 'table',
                    kind: 'table',
                    text: '',
                    data: {
                      'rows': [
                        ['`command` then \\`literal\\`', '**bold**'],
                        ['row', 'end'],
                      ],
                    },
                  ),
                ]
              : [
                  EditorNode(
                    id: 'code',
                    kind: 'code_block',
                    text: 'print(1)\nprint(2)',
                    data: {'language': 'py'},
                  ),
                  EditorNode(id: 'after', kind: 'paragraph', text: 'after'),
                ],
          version: 0,
          canEdit: true,
          onApplyOperations: (_) async {},
          commandHook: commands,
        ),
      ),
    ),
  );
  WidgetsBinding.instance.addPostFrameCallback((_) {
    if (isRenderFixture) {
      _hasHanGlyphs =
          (() => _distinctHanGlyphs().then((value) => value.toJS).toJS).toJS;
    }
    _mathState = (() {
      final images = <String, int>{};
      final folded = <String>{};
      final previews = <String, int>{};
      void visit(Element element) {
        final widget = element.widget;
        if (widget is DocumentSurface) {
          for (final entry
              in (widget.previewImages['math-inline'] ?? {}).entries) {
            images[entry.key] = entry.value.width;
          }
          for (final kind in ['math', 'mermaid']) {
            previews[kind] = widget.previewImages[kind]?.length ?? 0;
          }
          final render =
              (element as RenderObjectElement).renderObject as RenderDocument;
          // Hit-testing only succeeds on a folded formula's painted rectangle.
          // A raster in the cache alone does not prove the editor displays it.
          for (var y = 0.0; y < 40; y += 4) {
            for (var x = 0.0; x < render.size.width; x += 4) {
              final hit = render.inlineMathAt(Offset(x, y));
              if (hit != null) folded.add(hit.source);
            }
          }
        }
        element.visitChildElements(visit);
      }

      final context = editorKey.currentContext;
      if (context is Element) visit(context);
      return jsonEncode({
        'images': images,
        'folded': folded.toList(),
        'previews': previews,
      }).toJS;
    }).toJS;
    _focusEditor = (() {
      commands.focusFirstLine();
      return true.toJS;
    }).toJS;
  });
}

// Same layout, different Han glyph. Tofu would give two identical images.
Future<bool> _distinctHanGlyphs() async {
  final images = await Future.wait([
    for (final label in ['一', '三'])
      renderMermaid('flowchart LR\n  A[$label]', 140, MicaTokens.light),
  ]);
  try {
    final [a, b] = images;
    if (a == null || b == null || a.width != b.width || a.height != b.height) {
      return false;
    }
    final left = await a.toByteData(format: ui.ImageByteFormat.rawRgba);
    final right = await b.toByteData(format: ui.ImageByteFormat.rawRgba);
    return left != null &&
        right != null &&
        !listEquals(left.buffer.asUint8List(), right.buffer.asUint8List());
  } finally {
    for (final image in images) {
      image?.dispose();
    }
  }
}

// Render the same source through the real editor in Playwright.
List<EditorNode> _renderFixture() {
  const prefix = 'Inline fraction: ';
  const inline = r'\frac{a}{b}';
  return [
    EditorNode(
      id: 'inline-math',
      kind: 'paragraph',
      text: '$prefix$inline',
      data: {
        'marks': marksToJson([
          Mark(prefix.length, prefix.length + inline.length, 'math'),
        ]),
      },
    ),
    EditorNode(
      id: 'display-math',
      kind: 'math_block',
      text: r'\sum_{n=1}^\infty \frac{1}{n^2} = \frac{\pi^2}{6}',
    ),
    EditorNode(
      id: 'mermaid',
      kind: 'code_block',
      text: 'flowchart LR\n  A[开始] --> B{确认}\n  B --> C[完成]',
      data: {'language': 'mermaid'},
    ),
    EditorNode(
      id: 'after',
      kind: 'paragraph',
      text: 'The source remains editable.',
    ),
  ];
}
