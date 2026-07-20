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
  return UpdateInfo(
    version: latest,
    downloadUrl: url,
    notes: (json['body'] as String?)?.trim(),
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
  // `ping` is the delay: `timeout /t` wants a console this detached process has
  // no claim on. /CLOSEAPPLICATIONS stays as a backstop for a slow exit, and
  // /SUPPRESSMSGBOXES is deliberately gone — if Setup still cannot proceed, a
  // visible prompt beats a silent rollback. The log gives us something to read
  // when someone reports "it didn't update".
  final logPath = '${dir.path}\\inno-install.log';
  await Process.start(
    'cmd',
    [
      '/c',
      'ping 127.0.0.1 -n 4 >nul & '
          '"${setup.path}" /VERYSILENT /NOCANCEL /CLOSEAPPLICATIONS '
          '/NORESTART "/LOG=$logPath"',
    ],
    mode: ProcessStartMode.detached,
  );

  // Quit immediately: every millisecond spent here is one Setup may have to
  // spend prying the files loose.
  exit(0);
}
