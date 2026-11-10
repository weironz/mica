// The code block's title dialog, split out of the editor library (its size
// ratchet): self-contained, no editor state.
import 'package:flutter/material.dart';

import '../l10n/locale_controller.dart';

/// The code-block title dialog, owning its controller. Disposing a controller
/// right after showDialog returns throws in debug (`_dependents.isEmpty`): the
/// pop only starts the exit transition, which rebuilds the field and re-adds a
/// listener to the just-freed controller. State.dispose runs after the route is
/// gone, so it's the safe hook.
class CodeTitleDialog extends StatefulWidget {
  const CodeTitleDialog({super.key, required this.initial});

  final String initial;

  @override
  State<CodeTitleDialog> createState() => _CodeTitleDialogState();
}

class _CodeTitleDialogState extends State<CodeTitleDialog> {
  late final TextEditingController _ctrl = TextEditingController(
    text: widget.initial,
  );

  @override
  void dispose() {
    _ctrl.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final l10n = context.l10n;
    return AlertDialog(
      title: Text(l10n.codeMenuAddTitle),
      content: TextField(
        controller: _ctrl,
        autofocus: true,
        decoration: InputDecoration(
          hintText: l10n.codeTitleHint,
          border: const OutlineInputBorder(),
        ),
        onSubmitted: (v) => Navigator.of(context).pop(v),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: Text(l10n.commonCancel),
        ),
        FilledButton(
          onPressed: () => Navigator.of(context).pop(_ctrl.text),
          child: Text(l10n.commonSave),
        ),
      ],
    );
  }
}
