import 'package:flutter/material.dart';

import '../editor/model.dart';
import '../editor/render.dart';
import 'theme_tokens.dart';

/// A read-only sample using the editor's actual typography. It cannot take
/// editing focus, mutate a document, or start any data/synchronization work.
class MicaAppearancePreview extends StatelessWidget {
  const MicaAppearancePreview({
    required this.appearance,
    required this.pageWidth,
    required this.title,
    required this.body,
    required this.code,
    required this.label,
    super.key,
  });

  final EditorAppearance appearance;
  final double pageWidth;
  final String title;
  final String body;
  final String code;
  final String label;

  @override
  Widget build(BuildContext context) {
    final tokens = MicaTheme.of(context);
    TextStyle style(String kind) => appearance.applyTo(
      EditorTheme.styleFor(
        EditorNode(id: 'preview-$kind', kind: kind, text: ''),
        tokens,
      ),
      isCode: kind == 'code_block',
    );
    return Container(
      key: const ValueKey('settings-appearance-preview'),
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: tokens.surface.base,
        border: Border.all(color: tokens.border.normal),
        borderRadius: BorderRadius.circular(10),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            '$label · ${pageWidth.round()} px',
            style: TextStyle(fontSize: 12, color: tokens.text.muted),
          ),
          const SizedBox(height: 18),
          LayoutBuilder(
            builder: (context, constraints) => FractionallySizedBox(
              widthFactor: constraints.maxWidth < 400
                  ? 1
                  : (pageWidth / 1200).clamp(0.55, 1.0),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(title, style: style('heading')),
                  const SizedBox(height: 12),
                  Text(body, style: style('paragraph')),
                  const SizedBox(height: 14),
                  Container(
                    padding: const EdgeInsets.all(12),
                    decoration: BoxDecoration(
                      color: tokens.editor.codeBg,
                      borderRadius: BorderRadius.circular(8),
                    ),
                    child: Text(code, style: style('code_block')),
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}
