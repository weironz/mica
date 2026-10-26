/// Desktop self-updater: checks GitHub Releases, downloads the installer, and
/// launches it silently so it force-closes this app, installs, and relaunches.
///
/// Windows-only in practice — it drives the Inno Setup installer
/// (`Mica-Setup-*.exe`). macOS/Linux have no packaged installer, so
/// [updateSupported] is false there and [downloadAndApplyUpdate] refuses.
library;

import 'dart:convert';
import 'dart:io';

import 'package:http/http.dart' as http;
import 'package:flutter/foundation.dart' show visibleForTesting;

import 'l10n/locale_controller.dart';
import 'updater_common.dart';
import 'upload/sha256.dart';
import 'window_setup_desktop.dart' show flushBeforeExit;

export 'updater_common.dart';

/// Whether this build can update itself in place (only the Windows installer).
bool get updateSupported => Platform.isWindows;

/// How long the installer download may wait for the server to send response
/// headers before the attempt is abandoned. A server that accepts the TCP
/// connection and then says nothing used to hang the download page forever.
const Duration kUpdateResponseTimeout = Duration(seconds: 20);

/// The longest allowed GAP between body chunks. Deliberately an idle timeout,
/// not a total deadline: a large installer on a slow-but-progressing link must
/// finish, while a connection that stalls mid-body must fail promptly.
const Duration kUpdateBodyIdleTimeout = Duration(seconds: 30);

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

/// Download the installer into a fresh temp dir and verify it, returning the
/// verified file. The dir is deleted on EVERY failure — network error, timeout,
/// non-200, write error, integrity mismatch — so a retry loop cannot fill TEMP
/// with half-installers. Only a caller that successfully hands the file to the
/// installer (`downloadAndApplyUpdate`) keeps it.
///
/// `responseTimeout`/`idleTimeout` are overridable so tests can exercise the
/// failure modes in milliseconds instead of half a minute.
Future<File> _downloadInstaller(
  UpdateInfo info, {
  void Function(double progress)? onProgress,
  Duration responseTimeout = kUpdateResponseTimeout,
  Duration idleTimeout = kUpdateBodyIdleTimeout,
}) async {
  final dir = await Directory.systemTemp.createTemp('mica_update_');
  final setup = File('${dir.path}/Mica-Setup-${info.version}.exe');

  final client = http.Client();
  var keep = false;
  try {
    final resp = await client
        .send(http.Request('GET', Uri.parse(info.downloadUrl)))
        .timeout(responseTimeout);
    if (resp.statusCode != 200) {
      throw Exception(l10nNoContext.updaterDownloadFailed(resp.statusCode));
    }
    final total = resp.contentLength ?? 0;
    var received = 0;
    final file = await setup.open(mode: FileMode.write);
    try {
      // Reset the timeout per chunk so progressing downloads have no total
      // deadline. Await writes so disk errors propagate through this attempt.
      await for (final chunk in resp.stream.timeout(idleTimeout)) {
        await file.writeFrom(chunk);
        received += chunk.length;
        if (total > 0) onProgress?.call(received / total);
      }
    } finally {
      await file.close();
    }

    // Verify BEFORE running it: this launches an installer with the user's
    // privileges, so a truncated or swapped file must be rejected, never run.
    final bytes = await setup.readAsBytes();
    if (!installerMatches(bytes, size: info.size, sha256: info.sha256)) {
      throw UpdateIntegrityException(l10nNoContext.updaterIntegrityFailed);
    }
    keep = true;
    return setup;
  } finally {
    client.close();
    if (!keep) await _removeDir(dir);
  }
}

/// Launches the installer (a seam so failure-path cleanup is testable).
typedef InstallerLauncher =
    Future<void> Function(String path, List<String> args);

/// Ends the process (a seam so tests do not kill their own runner).
typedef ProcessQuitter = void Function(int code);

/// Download the installer to a temp file (reporting 0..1 progress), then launch
/// it silently and quit. The installer closes this app (already exiting),
/// installs the new version, and relaunches it.
Future<void> downloadAndApplyUpdate(
  UpdateInfo info, {
  void Function(double progress)? onProgress,
  Duration responseTimeout = kUpdateResponseTimeout,
  Duration idleTimeout = kUpdateBodyIdleTimeout,
  @visibleForTesting InstallerLauncher? launcher,
  @visibleForTesting ProcessQuitter? quit,
}) async {
  if (!Platform.isWindows) {
    throw UnsupportedError(l10nNoContext.updaterWindowsOnly);
  }

  final setup = await _downloadInstaller(
    info,
    onProgress: onProgress,
    responseTimeout: responseTimeout,
    idleTimeout: idleTimeout,
  );
  final dir = setup.parent;

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
  try {
    final logPath = '${dir.path}\\inno-install.log';
    // Persist edits BEFORE Setup starts its countdown on our PID.
    await flushBeforeExit();
    await (launcher ?? _startInstaller)(
      setup.path,
      setupArgs(logPath: logPath, waitPid: pid),
    );
  } catch (_) {
    // The installer never started, so this download is useless: do not leave it
    // where a later attempt (or a virus scanner) has to deal with it.
    await _removeDir(dir);
    rethrow;
  }

  // Quit immediately: every millisecond spent here is one Setup may have to
  // spend waiting on our PID.
  (quit ?? exit)(0);
}

Future<void> _startInstaller(String path, List<String> args) async {
  await Process.start(path, args, mode: ProcessStartMode.detached);
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
/// and [sha256] (lowercase hex). Pure (no I/O), so the integrity gate that
/// stands between a network download and running an .exe is unit-tested
/// directly. `size` rejects a truncated download; `sha256` rejects a corrupted
/// or swapped one.
///
/// **A missing [sha256] is a REFUSAL, not a downgrade.** It used to mean "skip
/// that check", which quietly reduced the gate to `size` alone — and size only
/// catches truncation: a file of identical length with different contents walks
/// straight through, which is the exact thing this gate exists to stop.
///
/// The digest comes from GitHub's `assets[].digest`, and today it is always
/// populated (verified 2026-08-05: all four assets of v0.13.13 carry one). That
/// is the point — the double check currently works BY ACCIDENT, and if GitHub
/// ever stopped filling the field nothing would announce the downgrade.
/// Refusing makes the dependency explicit: the updater either verifies the
/// installer or declines to run it, and the user updates by hand.
///
/// Publishing our own `SHA256SUMS` from CI was considered and rejected: it would
/// share a trust root with the asset it vouches for (both come from the same
/// release), so it adds no security. What would is signing the installer — a
/// separate item, deliberately not done (it needs a code-signing certificate).
bool installerMatches(List<int> bytes, {int? size, String? sha256}) {
  if (size != null && bytes.length != size) return false;
  if (sha256 == null) return false;
  return sha256Hex(bytes) == sha256;
}

/// Best-effort delete of one attempt's temp dir, so neither a rejected
/// installer nor a partial download is left where a later run might pick it up.
Future<void> _removeDir(Directory dir) async {
  try {
    await dir.delete(recursive: true);
  } catch (_) {}
}
