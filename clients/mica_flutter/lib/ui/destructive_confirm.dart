import 'package:flutter/material.dart';

import 'theme_tokens.dart';

/// The one red used for irreversible actions across the app.
///
/// A function, not a constant: the red is `status.danger`, and which red that is
/// depends on the palette in effect — light mode's #DC2626 is too dark to read on
/// a dark page.
Color kDestructiveRed(BuildContext context) =>
    MicaTheme.of(context).status.danger;

/// The confirmation gate in front of an action that cannot be undone.
///
/// It lives in its own library, rather than as a helper inside `dialogs.dart`,
/// for two reasons. `dialogs.dart` is `part of main.dart`, so nothing in it can
/// be constructed from a test — and "does the destructive path actually stop and
/// ask?" is precisely the behaviour that must not regress silently. Second, the
/// app had grown several hand-rolled confirm dialogs that each re-decided
/// whether the confirm button should look dangerous, while two genuinely
/// irreversible actions — purging from the recycle bin, revoking an API token —
/// had skipped the question altogether.
///
/// Deliberately not localized here: the caller passes finished strings, the same
/// contract `status_kit.dart` uses, so this library stays independent of the
/// generated `AppLocalizations`.
///
/// [destructive] false keeps the gate but drops the red. Some actions need to be
/// confirmed without being irreversible — clearing the re-downloadable half of
/// the on-device cache costs you offline access until you reconnect, and nothing
/// else. Painting that the same colour as "purge the recycle bin" would spend the
/// red on something recoverable, and the red only means anything while it is
/// reserved for what it says.
Future<bool> showDestructiveConfirm(
  BuildContext context, {
  required String title,
  required String body,
  required String confirmLabel,
  required String cancelLabel,
  bool destructive = true,
}) async {
  final confirmed = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(title),
      content: Text(body),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: Text(cancelLabel),
        ),
        FilledButton(
          style: destructive
              ? FilledButton.styleFrom(
                  backgroundColor: kDestructiveRed(context),
                )
              : null,
          onPressed: () => Navigator.pop(context, true),
          child: Text(confirmLabel),
        ),
      ],
    ),
  );
  // Dismissing by tapping outside or pressing Esc yields null, and null has to
  // read as "no" — that is the entire point of the gate.
  return confirmed == true;
}
