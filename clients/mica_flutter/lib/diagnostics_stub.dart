/// Opt-in capture of what the app actually RECEIVED, for reproducing a bug.
///
/// Off by default; the user turns it on in Settings → 诊断 before reproducing,
/// then hands over the folder. It exists because the expensive part of a bug
/// report is rarely the fix — it is not being able to see the input. A pasted
/// list once vanished through a whole afternoon of guessing, and five
/// hand-built HTML samples all converted correctly; the real clipboard markup
/// turned out to wrap every `<li>` in a `<div>`, which nobody would think to
/// guess. Capturing the input would have made it a two-minute fix.
///
/// Deliberately NOT a verbose log. Two kinds of record only:
///   * the raw INPUT of a conversion (clipboard HTML) with what it became, and
///   * one line per key DECISION (a bootstrap's block count and its source),
///     which is what turns "the page is blank" into "the server sent 2 blocks
///     and the local copy has 114".
/// Everything else is noise that buries the signal — and is usually switched
/// off when it finally matters.
library;

import 'dart:io';

import 'prefs.dart';

/// How many capture files to keep. Small on purpose: a reproduction is a few
/// actions, and an unbounded folder in the user's profile is its own bug.
const int _keep = 20;

/// Whether this build can capture at all. False on web (no filesystem), where
/// Settings hides the section instead of showing a switch that does nothing.
const bool diagnosticsSupported = true;

const String _prefKey = 'diagnostics';

/// Whether capture is on. Read on every capture so the toggle takes effect
/// immediately, with no restart.
bool get diagnosticsOn => loadPref(_prefKey) == 'true';

void setDiagnostics(bool on) => savePref(_prefKey, on ? 'true' : 'false');

/// Where captures land — shown in Settings so the user can find them.
String get diagnosticsDir => '${configDir()}/debug';

/// Record the input and the output of one conversion.
///
/// [kind] names the capture (`paste`), [ext] the input's extension (`html`).
/// The converted result is written beside it as `.md`, so a report carries both
/// halves and the pair can be replayed directly in a test.
void captureIo(String kind, String ext, String input, String output) {
  if (!diagnosticsOn) return;
  try {
    final dir = Directory(diagnosticsDir)..createSync(recursive: true);
    final stamp =
        DateTime.now().toIso8601String().replaceAll(':', '').replaceAll('.', '-');
    File('${dir.path}/$kind-$stamp.$ext').writeAsStringSync(input);
    File('${dir.path}/$kind-$stamp.md').writeAsStringSync(output);
    _prune(dir);
  } catch (_) {
    // Diagnostics must never be able to break the thing being diagnosed.
  }
}

/// Append an uncaught error + stack to `crash.log`. The ONE record here that is
/// deliberately NOT gated by [diagnosticsOn]: a crash is exactly the case the
/// opt-in capture can't be armed for in advance, and the volume is negligible
/// (a few lines only when the app actually faults). Best-effort and
/// self-silencing — a diagnostics write must never be able to worsen a crash —
/// and self-bounding so a crash loop can't fill the user's disk.
void logCrash(String message) {
  try {
    final dir = Directory(diagnosticsDir)..createSync(recursive: true);
    final file = File('${dir.path}/crash.log');
    try {
      if (file.existsSync() && file.lengthSync() > 256 * 1024) {
        file.writeAsStringSync(''); // truncate a runaway log
      }
    } catch (_) {
      // A stat/truncate hiccup is not worth losing the crash line over.
    }
    file.writeAsStringSync(
      '${DateTime.now().toIso8601String()}  $message\n',
      mode: FileMode.append,
    );
  } catch (_) {
    // As with the rest of this file: never break the thing being diagnosed.
  }
}

/// Append one line to `trace.log` — a decision worth being able to look back at.
void trace(String line) {
  if (!diagnosticsOn) return;
  try {
    final dir = Directory(diagnosticsDir)..createSync(recursive: true);
    File('${dir.path}/trace.log').writeAsStringSync(
      '${DateTime.now().toIso8601String()}  $line\n',
      mode: FileMode.append,
    );
  } catch (_) {
    // As above.
  }
}

/// Reveal the folder in the OS file manager (Settings' "open folder").
Future<void> openDiagnosticsFolder() async {
  try {
    Directory(diagnosticsDir).createSync(recursive: true);
    final path = diagnosticsDir.replaceAll('/', Platform.pathSeparator);
    if (Platform.isWindows) {
      await Process.run('explorer.exe', [path]);
    } else if (Platform.isMacOS) {
      await Process.run('open', [path]);
    } else {
      await Process.run('xdg-open', [path]);
    }
  } catch (_) {
    // Nothing to do — the path is shown next to the button anyway.
  }
}

/// Keep the newest [_keep] captures. `trace.log` and `crash.log` are exempt:
/// each is one self-bounding file, appended a line at a time — and pruning the
/// crash log (which capture doesn't produce) would defeat its whole purpose.
void _prune(Directory dir) {
  final files = dir
      .listSync()
      .whereType<File>()
      .where((f) =>
          !f.path.endsWith('trace.log') && !f.path.endsWith('crash.log'))
      .toList()
    ..sort((a, b) => a.path.compareTo(b.path));
  final excess = files.length - _keep;
  if (excess <= 0) return;
  for (final f in files.take(excess)) {
    try {
      f.deleteSync();
    } catch (_) {
      // A file held open in the user's editor is not worth failing over.
    }
  }
}
