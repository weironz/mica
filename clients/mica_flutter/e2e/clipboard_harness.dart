// Browser-only regression fixture. It renders the real editor and exposes only
// a focus action; selection and copy still travel through real keyboard events.
import 'dart:js_interop';

import 'package:flutter/material.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/l10n/app_localizations.dart';

@JS('micaClipboardHarnessFocus')
external set _focusEditor(JSFunction callback);

void main() {
  final commands = EditorCommandHook();
  runApp(
    MaterialApp(
      localizationsDelegates: AppLocalizations.localizationsDelegates,
      supportedLocales: AppLocalizations.supportedLocales,
      home: Scaffold(
        body: MicaEditor(
          rootBlockId: 'root',
          nodes: [
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
    _focusEditor = (() {
      commands.focusFirstLine();
      return true.toJS;
    }).toJS;
  });
}
