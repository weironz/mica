// The desktop updater's DOWNLOAD half, against a real local HTTP server.
//
// Two failure modes this pins (issue #140):
//  1. no response-header / body-idle timeout: a server that accepts the
//     connection and never answers, or answers 200 and then stops sending,
//     left the download page spinning forever with no cancel/retry;
//  2. every failed attempt leaked its `mica_update_*` temp dir — and often a
//     partial `.exe` — because only the integrity branch called `_discard`.
//
// The timeout is an IDLE timeout on the body, not a total deadline: a slow but
// progressing download must still finish (covered below), because a total
// timeout would truncate exactly the connections it was meant to protect.
@TestOn('windows')
@Timeout(Duration(minutes: 3))
library;

import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:mica_flutter/updater.dart';
import 'package:mica_flutter/upload/sha256.dart';

const _body = 'installer-bytes';

UpdateInfo infoFor(Uri url, {List<int>? bytes}) {
  final data = bytes ?? Uint8List.fromList(_body.codeUnits);
  return UpdateInfo(
    version: '9.9.9',
    downloadUrl: url.toString(),
    size: data.length,
    sha256: sha256Hex(data),
  );
}

/// Temp dirs the updater may have left behind. `Directory.systemTemp` cannot be
/// re-pointed per test, so compare snapshots around each attempt instead.
Set<String> updateTempDirs() => Directory.systemTemp
    .listSync()
    .whereType<Directory>()
    .map((d) => d.path)
    .where(
      (p) => p.split(Platform.pathSeparator).last.startsWith('mica_update_'),
    )
    .toSet();

void main() {
  // The failure paths build localized messages through `l10nNoContext`, which
  // needs a binding. flutter_test installs an HttpClient mock that answers
  // every request with 400; clear it so the download really hits our loopback
  // server (the point of these tests is the real wire behaviour).
  TestWidgetsFlutterBinding.ensureInitialized();
  HttpOverrides.global = null;

  HttpServer? server;
  late Uri url;

  Future<void> serve(FutureOr<void> Function(HttpRequest) handler) async {
    final s = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server = s;
    s.listen(handler);
    url = Uri.parse('http://127.0.0.1:${s.port}/Mica-Setup-9.9.9.exe');
  }

  tearDown(() async => server?.close(force: true));

  const headerTimeout = Duration(milliseconds: 250);
  const idleTimeout = Duration(milliseconds: 250);

  test('a server that accepts but never sends headers times out', () async {
    final before = updateTempDirs();
    await serve((req) {
      // Hold the request open forever: connection accepted, no headers.
    });

    await expectLater(
      downloadAndApplyUpdate(
        infoFor(url),
        responseTimeout: headerTimeout,
        idleTimeout: idleTimeout,
      ).timeout(
        const Duration(minutes: 2),
        onTimeout: () {
          throw StateError(
            'updater did not enforce its response-header timeout',
          );
        },
      ),
      throwsA(isA<TimeoutException>()),
      reason: 'without a header timeout this await never returns',
    );
    expect(
      updateTempDirs(),
      before,
      reason: 'the failed attempt must not leave a temp dir or partial exe',
    );
  });

  test('headers then a stalled body times out on the idle gap', () async {
    final before = updateTempDirs();
    await serve((req) async {
      req.response.contentLength = 1000;
      req.response.add(Uint8List.fromList([1, 2, 3]));
      await req.response.flush();
      // ...and then nothing, ever. The connection stays open.
    });

    await expectLater(
      downloadAndApplyUpdate(
        infoFor(url),
        responseTimeout: headerTimeout,
        idleTimeout: idleTimeout,
      ).timeout(
        const Duration(minutes: 2),
        onTimeout: () {
          throw StateError('updater did not enforce its body idle timeout');
        },
      ),
      throwsA(isA<TimeoutException>()),
    );
    expect(updateTempDirs(), before);
  });

  test('a body connection dropped mid-download fails and cleans up', () async {
    final before = updateTempDirs();
    // HttpServer refuses `detachSocket` once headers are out, so speak the few
    // bytes of HTTP needed to promise a body and then abort the TCP connection.
    final raw = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    raw.listen((socket) {
      socket.listen((_) async {
        socket.write('HTTP/1.1 200 OK\r\nContent-Length: 100000\r\n\r\n');
        socket.add(Uint8List(4096));
        await socket.flush();
        socket.destroy();
      });
    });
    final rawUrl = Uri.parse(
      'http://127.0.0.1:${raw.port}/Mica-Setup-9.9.9.exe',
    );

    try {
      await expectLater(
        downloadAndApplyUpdate(
          infoFor(rawUrl),
          responseTimeout: headerTimeout,
          idleTimeout: idleTimeout,
        ),
        throwsA(isA<http.ClientException>()),
        reason: 'a dropped connection must surface as a network error',
      );
      expect(updateTempDirs(), before);
    } finally {
      await raw.close();
    }
  });

  test('a slow but progressing download outlives the header timeout', () async {
    final data = Uint8List.fromList(List.generate(4096 * 8, (i) => i & 0xff));
    await serve((req) async {
      req.response.contentLength = data.length;
      const chunk = 4096;
      for (var off = 0; off < data.length; off += chunk) {
        req.response.add(Uint8List.sublistView(data, off, off + chunk));
        await req.response.flush();
        // 8 chunks x 60 ms = ~0.5 s total, far past the 100 ms header timeout,
        // while every gap stays under the idle timeout.
        await Future<void>.delayed(const Duration(milliseconds: 60));
      }
      await req.response.close();
    });

    File? file;
    await downloadAndApplyUpdate(
      infoFor(url, bytes: data),
      responseTimeout: const Duration(milliseconds: 100),
      idleTimeout: const Duration(milliseconds: 400),
      launcher: (path, args) async => file = File(path),
      quit: (_) {},
    );
    expect(
      await file!.readAsBytes(),
      data,
      reason: 'an idle timeout must not truncate a slow, live download',
    );
    expect(file!.parent.path, contains('mica_update_'));
    await file!.parent.delete(recursive: true);
  });

  test('non-200 fails before writing and leaves nothing behind', () async {
    final before = updateTempDirs();
    await serve((req) async {
      req.response.statusCode = 404;
      await req.response.close();
    });

    await expectLater(
      downloadAndApplyUpdate(infoFor(url)),
      throwsA(isA<Exception>()),
    );
    expect(updateTempDirs(), before);
  });

  test(
    'installer file creation failure cleans its attempt directory',
    () async {
      final before = updateTempDirs();
      await serve((req) async {
        req.response.add(Uint8List.fromList(_body.codeUnits));
        await req.response.close();
      });
      await expectLater(
        downloadAndApplyUpdate(
          UpdateInfo(
            version: 'missing/subdirectory',
            downloadUrl: url.toString(),
            size: _body.length,
            sha256: sha256Hex(_body.codeUnits),
          ),
        ),
        throwsA(isA<FileSystemException>()),
      );
      expect(updateTempDirs(), before);
    },
  );

  test('integrity failure deletes the installer and its dir', () async {
    final before = updateTempDirs();
    await serve((req) async {
      req.response.add(Uint8List.fromList(_body.codeUnits));
      await req.response.close();
    });

    await expectLater(
      downloadAndApplyUpdate(
        UpdateInfo(
          version: '9.9.9',
          downloadUrl: url.toString(),
          size: _body.length,
          sha256: 'deadbeef',
        ),
      ),
      throwsA(isA<UpdateIntegrityException>()),
    );
    expect(
      updateTempDirs(),
      before,
      reason: 'a rejected installer must never be left for a later run',
    );
  });

  test(
    'launch failure after a good download removes the handed-off dir',
    () async {
      final before = updateTempDirs();
      await serve((req) async {
        req.response.add(Uint8List.fromList(_body.codeUnits));
        await req.response.close();
      });
      var quit = false;

      await expectLater(
        downloadAndApplyUpdate(
          infoFor(url),
          launcher: (path, args) async => throw StateError('spawn failed'),
          quit: (_) => quit = true,
        ),
        throwsA(isA<StateError>()),
      );
      expect(quit, isFalse, reason: 'nothing was installed, so do not exit');
      expect(
        updateTempDirs(),
        before,
        reason: 'a dir is kept only after the installer is actually launched',
      );
    },
  );

  test(
    'successful handoff keeps the dir and the installer for Setup',
    () async {
      await serve((req) async {
        req.response.add(Uint8List.fromList(_body.codeUnits));
        await req.response.close();
      });
      File? launched;
      List<String>? launchedArgs;
      var quitCode = -1;

      await downloadAndApplyUpdate(
        infoFor(url),
        launcher: (path, args) async {
          launched = File(path);
          launchedArgs = args;
        },
        quit: (code) => quitCode = code,
      );

      expect(launched, isNotNull);
      expect(await launched!.readAsString(), _body);
      expect(launched!.parent.path, contains('mica_update_'));
      expect(launchedArgs, contains('/VERYSILENT'));
      expect(launchedArgs, contains('/MICAWAITPID=$pid'));
      expect(
        launchedArgs,
        contains('/LOG=${launched!.parent.path}\\inno-install.log'),
      );
      expect(
        quitCode,
        0,
        reason: 'Setup waits on our PID, so the app must exit after launching',
      );
      await launched!.parent.delete(recursive: true);
    },
  );
}
