import 'dart:io';

/// Open [url] in the user's default browser by handing off to the OS shell —
/// in-house, no url_launcher dependency. Best-effort: a failed handoff (e.g. on
/// a platform without these openers) degrades to the prior no-op behaviour.
void openUrl(String url) {
  try {
    if (Platform.isWindows) {
      // explorer.exe routes http(s) URLs to the default browser and, unlike
      // `cmd /c start`, handles `&` in query strings without shell quoting.
      Process.run('explorer.exe', [url]);
    } else if (Platform.isMacOS) {
      Process.run('open', [url]);
    } else {
      Process.run('xdg-open', [url]);
    }
  } catch (_) {
    // Nothing sensible to do if the shell handoff itself throws.
  }
}
