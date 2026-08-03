// A copy control that answers "did that work?" where the cursor already is.
//
// The alternative — and what this replaces everywhere it is used — is a
// SnackBar: a black bar thrown across the bottom of the window to report that
// the expected thing happened. It is the loudest possible way to say the least,
// it covers content, and it appears nowhere near the button you just pressed.
//
// Its own library rather than a member of `ui/widgets.dart`, because that file
// is `part of main.dart` and the editor (`lib/editor/`) is a separate library
// that needs this too.
import 'dart:async';

import 'package:flutter/material.dart';

import '../l10n/locale_controller.dart';
import 'theme_tokens.dart';

/// Green-600. "It worked" — deliberately not the UI accent, which means
/// "selected/active" everywhere else in this app.
const kCopyDoneColor = Color(0xFF16A34A);

/// How long the confirmation stays. Long enough to be seen, short enough that
/// the button is back to its real affordance before you next look at it.
const kCopyDoneDuration = Duration(milliseconds: 1600);

class InlineCopyButton extends StatefulWidget {
  const InlineCopyButton({
    required this.onCopy,
    required this.tooltip,
    this.label,
    this.size = 13,
    super.key,
  });

  /// Returns whether the copy succeeded. False leaves the button alone — the
  /// check is the only signal the user gets, so it has to mean exactly one
  /// thing. A refusal still needs words, and the caller says them.
  final Future<bool> Function() onCopy;

  /// Idle tooltip. Replaced by "copied" while the check is showing.
  final String tooltip;

  /// Non-null gives the labelled shape (a `TextButton.icon`), for the dialogs
  /// where the control sits in a row of worded buttons. The label swaps to
  /// "copied" alongside the icon.
  final String? label;

  final double size;

  @override
  State<InlineCopyButton> createState() => _InlineCopyButtonState();
}

class _InlineCopyButtonState extends State<InlineCopyButton> {
  bool _done = false;
  Timer? _reset;

  @override
  void dispose() {
    _reset?.cancel();
    super.dispose();
  }

  Future<void> _run() async {
    final ok = await widget.onCopy();
    if (!mounted || !ok) return;
    setState(() => _done = true);
    _reset?.cancel();
    _reset = Timer(kCopyDoneDuration, () {
      if (mounted) setState(() => _done = false);
    });
  }

  @override
  Widget build(BuildContext context) {
    final icon = Icon(
      _done ? Icons.check : Icons.content_copy_outlined,
      size: widget.size,
    );

    final label = widget.label;
    if (label != null) {
      return TextButton.icon(
        onPressed: _run,
        icon: icon,
        label: Text(_done ? context.l10n.commonCopied : label),
        style: TextButton.styleFrom(
          foregroundColor: _done ? kCopyDoneColor : null,
        ),
      );
    }

    return IconButton(
      onPressed: _run,
      tooltip: _done ? context.l10n.commonCopied : widget.tooltip,
      visualDensity: VisualDensity.compact,
      padding: EdgeInsets.zero,
      constraints: const BoxConstraints.tightFor(width: 22, height: 22),
      color: _done ? kCopyDoneColor : MicaTheme.of(context).text.faint,
      icon: icon,
    );
  }
}
