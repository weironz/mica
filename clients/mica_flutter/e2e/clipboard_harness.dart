// Browser-only regression fixture. It renders the real editor and exposes only
// focus and rendering status; selection, copy and paste use real keyboard events.
import 'dart:js_interop';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/marks.dart';
import 'package:mica_flutter/editor/render.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

@JS('micaClipboardHarnessFocus')
external set _focusEditor(JSFunction callback);

@JS('micaClipboardHarnessMathState')
external set _mathState(JSFunction callback);

void main() {
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
          nodes: Uri.base.queryParameters['fixture'] == 'empty'
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
    _mathState = (() {
      final images = <String, int>{};
      final folded = <String>{};
      void visit(Element element) {
        final widget = element.widget;
        if (widget is DocumentSurface) {
          for (final entry in (widget.previewImages['math'] ?? {}).entries) {
            images[entry.key] = entry.value.width;
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
      return jsonEncode({'images': images, 'folded': folded.toList()}).toJS;
    }).toJS;
    _focusEditor = (() {
      commands.focusFirstLine();
      return true.toJS;
    }).toJS;
  });
}
