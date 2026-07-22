// The self-updater's post-exit script, EXECUTED — not just string-matched.
//
// The bug this pins: the script used to be a `cmd /c "ping … & "<setup>" …"`
// one-liner. Dart builds a Windows command line with the MSVC argv convention,
// which escapes an embedded `"` as `\"`; cmd.exe does not speak that convention
// and takes `\"` literally, so the installer path arrived as `\"C:\…exe\"` and
// the whole `&` chain died at that token. Setup never ran. Users saw a
// download, a restart, and the same old version — with a stray ping console as
// the only visible symptom.
//
// An assertion over the script TEXT would have passed the entire time: the text
// was fine, the argv round-trip was not. So these tests run the real script
// against a stub "installer" and check the stub actually executed.
@TestOn('windows')
library;

import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/updater.dart';

/// A stand-in for Mica-Setup-X.Y.Z.exe: records that it ran, and with what.
File _stubInstaller(Directory dir, String marker) {
  final stub = File('${dir.path}\\stub-setup.cmd')
    ..writeAsStringSync('@echo off\r\necho RAN %* > "$marker"\r\n');
  return stub;
}

Future<void> _run(File script) async {
  final r = await Process.run('cmd', ['/c', script.path]);
  expect(r.exitCode, 0, reason: 'script failed: ${r.stderr}');
}

void main() {
  late Directory dir;

  setUp(() => dir = Directory.systemTemp.createTempSync('mica_upd_test_'));
  tearDown(() {
    try {
      dir.deleteSync(recursive: true);
    } catch (_) {/* the stub may still be settling; the temp dir is disposable */}
  });

  test('the installer actually runs, with its flags intact', () async {
    final marker = '${dir.path}\\ran.txt';
    final stub = _stubInstaller(dir, marker);
    await _run(writeUpdateScript(dir, stub.path, '${dir.path}\\inno.log'));

    expect(File(marker).existsSync(), isTrue,
        reason: 'the installer never executed — the argv/cmd quoting round-trip '
            'is back (this is the exact regression: silent no-op update)');
    final got = File(marker).readAsStringSync();
    expect(got, contains('/VERYSILENT'));
    expect(got, contains('/CLOSEAPPLICATIONS'));
    expect(got, contains('/LOG='),
        reason: 'without the log there is nothing to read when a user reports '
            '"it did not update"');
  });

  test('a path with spaces survives — the case that broke it', () async {
    final spaced = Directory('${dir.path}\\Program Files Like This')
      ..createSync();
    final marker = '${spaced.path}\\ran.txt';
    final stub = _stubInstaller(spaced, marker);
    await _run(writeUpdateScript(dir, stub.path, '${spaced.path}\\inno.log'));

    expect(File(marker).existsSync(), isTrue,
        reason: 'a space in the installer path broke execution — quoting in the '
            'generated script is wrong');
  });

  test('forward slashes are normalised (the caller joins with "/")', () {
    final script =
        writeUpdateScript(dir, '${dir.path}/Mica-Setup-9.9.9.exe', 'C:/tmp/x.log');
    final text = script.readAsStringSync();

    expect(text, isNot(contains('/Mica-Setup')),
        reason: 'a mixed separator inside a quoted cmd token is fragile');
    expect(text, contains(r'\Mica-Setup-9.9.9.exe'));
    expect(text, contains(r'C:\tmp\x.log'));
    // The flags are the one place forward slashes are correct.
    expect(text, contains('/VERYSILENT'));
  });

  test('the console it opens is named, not anonymous', () {
    final script = writeUpdateScript(dir, 'C:\\x\\s.exe', 'C:\\x\\l.log');
    expect(script.readAsStringSync(), contains('title Mica'),
        reason: 'an unlabelled console mid-update reads as something strange '
            'happening to the machine');
  });

  // The integrity gate that stands between a network download and running an
  // .exe with the user's privileges. Known vector: sha256("abc").
  group('installerMatches', () {
    final bytes = utf8.encode('abc'); // 3 bytes
    const sha =
        'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad';

    test('accepts a matching size + sha256', () {
      expect(installerMatches(bytes, size: 3, sha256: sha), isTrue);
    });
    test('rejects a truncated download (size mismatch)', () {
      expect(installerMatches(bytes, size: 4, sha256: sha), isFalse);
    });
    test('rejects a swapped download (sha256 mismatch)', () {
      expect(installerMatches(bytes, size: 3, sha256: 'deadbeef'), isFalse);
    });
    test('falls back to size-only when the release has no digest', () {
      expect(installerMatches(bytes, size: 3), isTrue);
      expect(installerMatches(bytes, size: 4), isFalse);
    });
  });
}
