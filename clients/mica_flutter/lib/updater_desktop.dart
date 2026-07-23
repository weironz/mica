/// Desktop self-updater: checks GitHub Releases, downloads the installer, and
/// launches it silently so it force-closes this app, installs, and relaunches.
///
/// Windows-only in practice — it drives the Inno Setup installer
/// (`Mica-Setup-*.exe`). macOS/Linux have no packaged installer, so
/// [updateSupported] is false there and [downloadAndApplyUpdate] refuses.
library;

import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart' as crypto;
import 'package:http/http.dart' as http;

import 'l10n/locale_controller.dart';
import 'updater_common.dart';

export 'updater_common.dart';

/// Whether this build can update itself in place (only the Windows installer).
bool get updateSupported => Platform.isWindows;

/// Query GitHub for the latest release; return it only if it is newer than
/// [currentVersion] and ships a `Mica-Setup-*.exe` asset. Returns null when
/// already up to date. Throws on network / API failure so the UI can report it.
Future<UpdateInfo?> checkForUpdate(String currentVersion) async {
  final resp = await http
      .get(
        Uri.parse('https://api.github.com/repos/$kUpdateRepo/releases/latest'),
        headers: const {'Accept': 'application/vnd.github+json'},
      )
      .timeout(const Duration(seconds: 15));
  if (resp.statusCode != 200) {
    throw Exception(l10nNoContext.updaterGithubError(resp.statusCode));
  }
  final json = jsonDecode(resp.body) as Map<String, dynamic>;
  final tag = (json['tag_name'] as String?)?.trim() ?? '';
  final latest = tag.replaceFirst(RegExp(r'^[vV]'), '');
  if (latest.isEmpty || compareVersions(latest, currentVersion) <= 0) {
    return null; // no tag, or not newer → already up to date
  }
  final assets = (json['assets'] as List<dynamic>? ?? const [])
      .cast<Map<String, dynamic>>();
  Map<String, dynamic>? setup;
  for (final a in assets) {
    final name = (a['name'] as String? ?? '').toLowerCase();
    if (name.startsWith('mica-setup') && name.endsWith('.exe')) {
      setup = a;
      break;
    }
  }
  final url = setup?['browser_download_url'] as String?;
  if (url == null) return null; // release without a Windows installer asset
  // `digest` is GitHub's server-computed asset hash (`sha256:…`), present on
  // releases uploaded since GitHub added it; `size` is always present. Both feed
  // the pre-launch integrity check in downloadAndApplyUpdate.
  final digest = setup?['digest'] as String?;
  return UpdateInfo(
    version: latest,
    downloadUrl: url,
    notes: (json['body'] as String?)?.trim(),
    size: setup?['size'] as int?,
    sha256: (digest != null && digest.startsWith('sha256:'))
        ? digest.substring('sha256:'.length).toLowerCase()
        : null,
  );
}

/// Download the installer to a temp file (reporting 0..1 progress), then launch
/// it silently and quit. The installer closes this app (already exiting),
/// installs the new version, and relaunches it.
Future<void> downloadAndApplyUpdate(
  UpdateInfo info, {
  void Function(double progress)? onProgress,
}) async {
  if (!Platform.isWindows) {
    throw UnsupportedError(l10nNoContext.updaterWindowsOnly);
  }

  final dir = await Directory.systemTemp.createTemp('mica_update_');
  final setup = File('${dir.path}/Mica-Setup-${info.version}.exe');

  final client = http.Client();
  try {
    final resp = await client.send(http.Request('GET', Uri.parse(info.downloadUrl)));
    if (resp.statusCode != 200) {
      throw Exception(l10nNoContext.updaterDownloadFailed(resp.statusCode));
    }
    final total = resp.contentLength ?? 0;
    var received = 0;
    final sink = setup.openWrite();
    try {
      await for (final chunk in resp.stream) {
        sink.add(chunk);
        received += chunk.length;
        if (total > 0) onProgress?.call(received / total);
      }
    } finally {
      await sink.close();
    }

    // Verify BEFORE running it: this launches an installer with the user's
    // privileges, so a truncated or swapped file must be rejected, never run.
    final bytes = await setup.readAsBytes();
    if (!installerMatches(bytes, size: info.size, sha256: info.sha256)) {
      await _discard(setup);
      throw Exception(l10nNoContext.updaterIntegrityFailed);
    }
  } finally {
    client.close();
  }

  // Start Setup only AFTER this process is gone.
  //
  // The old code launched Setup immediately and quit 900 ms later, betting the
  // app would die before Setup enumerated its files. Losing that race was silent
  // and total: RestartManager found `mica_flutter` still holding the files,
  // spent 30 s failing to close it, and then — because /SUPPRESSMSGBOXES turns
  // the Abort/Retry/Ignore prompt into Abort — rolled the whole install back.
  // The user saw a download, a restart, and the same old version, with nothing
  // to explain it. (Observed: exit code 5, "Some applications could not be shut
  // down".)
  //
  // We can't let Setup close us: the app intercepts its own WM_CLOSE
  // (close-to-tray), so RestartManager (`/CLOSEAPPLICATIONS`) can't close it —
  // it would hang for 30 s and roll back. So we quit OURSELVES (exit(0) below)
  // and let Setup WAIT for our PID to vanish before it copies a file. That wait
  // lives in the installer's `[Code]` (PrepareToInstall → OpenProcess(SYNCHRONIZE)
  // + WaitForSingleObject on `/MICAWAITPID`); see installer/mica.iss.
  //
  // Launch Setup DIRECTLY — no cmd, no ping, no vbs. `Setup.exe` is a
  // GUI-subsystem program, so it opens NO console window; the old "ping
  // 127.0.0.1" window came entirely from the cmd wrapper. Process.start passes
  // an argv LIST (Setup speaks the MSVC argv convention, unlike cmd), so a
  // spaced path needs no hand-quoting. `/SUPPRESSMSGBOXES` stays gone — a visible
  // prompt beats a silent rollback; the log is there to read when someone
  // reports "it didn't update".
  final logPath = '${dir.path}\\inno-install.log';
  await Process.start(
    setup.path,
    setupArgs(logPath: logPath, waitPid: pid),
    mode: ProcessStartMode.detached,
  );

  // Quit immediately: every millisecond spent here is one Setup may have to
  // spend waiting on our PID.
  exit(0);
}

/// The command line the in-app updater hands the Inno installer. A LIST (argv),
/// not a shell string: `Setup.exe` parses the MSVC argv convention, so a spaced
/// path needs no hand-quoting and there is no `cmd` round-trip to mangle it
/// (the bug that once shipped `\"C:\…exe\"` as a literal token and no-op'd the
/// update — gone entirely now that no shell is involved).
///
/// `/MICAWAITPID` is the running app's process id; the installer's `[Code]`
/// waits for it to exit before copying files (see installer/mica.iss), which is
/// the native replacement for the old `ping` delay — it waits for the ACTUAL
/// exit, not a guessed 3 s, and opens no console. Exposed for testing.
List<String> setupArgs({required String logPath, required int waitPid}) => [
  '/VERYSILENT',
  '/NOCANCEL',
  // Harmless backstop; the /MICAWAITPID wait is the real gate (our close-to-tray
  // interception defeats RestartManager's graceful close anyway).
  '/CLOSEAPPLICATIONS',
  '/NORESTART',
  '/MICAWAITPID=$waitPid',
  '/LOG=$logPath',
];

/// Whether the downloaded installer [bytes] match the release's expected [size]
/// and [sha256] (lowercase hex). A null field is not checked — an older release
/// has no `digest`, so only its `size` gates. Pure (no I/O), so the integrity
/// gate that stands between a network download and running an .exe is unit-tested
/// directly. `size` rejects a truncated download; `sha256` rejects a corrupted
/// or swapped one.
bool installerMatches(List<int> bytes, {int? size, String? sha256}) {
  if (size != null && bytes.length != size) return false;
  if (sha256 != null && crypto.sha256.convert(bytes).toString() != sha256) {
    return false;
  }
  return true;
}

/// Best-effort delete of a rejected/failed installer download, so a corrupt file
/// is never left where a later run might pick it up.
Future<void> _discard(File f) async {
  try {
    if (await f.exists()) await f.delete();
  } catch (_) {}
}
