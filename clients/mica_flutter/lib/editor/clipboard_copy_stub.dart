import 'package:flutter/services.dart';

/// Copy [text] to the system clipboard via Flutter's framework platform
/// channel — in-house, no plugin. Returns false only if the channel throws.
Future<bool> copyTextToClipboard(String text) async {
  try {
    await Clipboard.setData(ClipboardData(text: text));
    return true;
  } catch (_) {
    return false;
  }
}
