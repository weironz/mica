import 'package:flutter/material.dart';

import '../l10n/locale_controller.dart';
import '../ui/typography.dart';
import 'model.dart' show kMonoFont;

/// Edit a block or inline LaTeX formula in a modal dialog.
Future<String?> showMathEditDialog(
  BuildContext context, {
  required String initial,
  required String hint,
}) async {
  final controller = TextEditingController(text: initial);
  try {
    return await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(context.l10n.slashMathFormula, style: MicaTypography.large),
        content: SizedBox(
          width: 420,
          child: TextField(
            controller: controller,
            autofocus: true,
            maxLines: 4,
            minLines: 1,
            style: MicaTypography.body.copyWith(fontFamily: kMonoFont),
            decoration: InputDecoration(hintText: hint, isDense: true),
          ),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(context.l10n.commonCancel),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: Text(context.l10n.commonOk),
          ),
        ],
      ),
    );
  } finally {
    controller.dispose();
  }
}

/// LaTeX sources are stored bare; users habitually paste `$$…$$`-wrapped
/// formulas (the app's own convention) which the typesetter can't parse.
String stripMathDelimiters(String source) {
  var src = source.trim();
  final m =
      RegExp(r'^\$\$(.*)\$\$$', dotAll: true).firstMatch(src) ??
      RegExp(r'^\\\[(.*)\\\]$', dotAll: true).firstMatch(src) ??
      RegExp(r'^\\\((.*)\\\)$', dotAll: true).firstMatch(src) ??
      RegExp(r'^\$([^$]+)\$$', dotAll: true).firstMatch(src);
  if (m != null) src = m.group(1)!.trim();
  return src;
}
