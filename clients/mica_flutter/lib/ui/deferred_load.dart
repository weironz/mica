import 'package:flutter/material.dart';
import '../l10n/locale_controller.dart';

/// A failed chunk download leaves the current page intact and can be retried
/// by reopening the command. No failed load is remembered as a content error.
Future<bool> loadUiModule(
  BuildContext context,
  Future<void> Function() load, {
  bool Function()? isCurrent,
}) async {
  try {
    await load();
    return context.mounted && (isCurrent?.call() ?? true);
  } catch (_) {
    if (context.mounted && (isCurrent?.call() ?? true)) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(SnackBar(content: Text(context.l10n.moduleLoadFailed)));
    }
    return false;
  }
}
