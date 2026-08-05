// The self-updater's launch contract, UNIT-tested.
//
// History: the launch used to be a `cmd /c "ping … & "<setup>" …"` one-liner,
// whose Dart→cmd argv round-trip mangled the quoted installer path (cmd does not
// speak the MSVC `\"` convention) and silently no-op'd the update, with a stray
// `ping` console as the only visible symptom. That whole class of bug is gone:
// the app now launches `Setup.exe` DIRECTLY with an argv LIST (no shell, no
// console window), and the installer waits for the app's PID to exit inside its
// own [Code] (installer/mica.iss). So there is no script to execute and no
// quoting round-trip — what's left to pin is the argument list and the
// integrity gate.
@TestOn('windows')
library;

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/updater.dart';

void main() {
  group('setupArgs', () {
    test('carries the silent-install flags + the PID to wait on + the log', () {
      final args = setupArgs(logPath: r'C:\tmp\inno.log', waitPid: 1234);
      expect(args, contains('/VERYSILENT'));
      expect(args, contains('/NORESTART'));
      // The PID is how the installer knows which process to wait for before it
      // touches a locked file — the native replacement for the old ping delay.
      expect(args, contains('/MICAWAITPID=1234'));
      expect(args, contains(r'/LOG=C:\tmp\inno.log'),
          reason: 'without the log there is nothing to read when a user reports '
              '"it did not update"');
    });

    test('a path with spaces needs no hand-quoting (argv, not a shell)', () {
      final args =
          setupArgs(logPath: r'C:\Program Files\Mica\inno.log', waitPid: 7);
      // The whole path is one argv element; Process.start quotes it for us and
      // Setup.exe parses the MSVC convention. No manual quotes in the value.
      expect(args, contains(r'/LOG=C:\Program Files\Mica\inno.log'));
      expect(args.any((a) => a.contains('"')), isFalse,
          reason: 'no shell round-trip means no hand-quoting — the old cmd '
              'quoting bug cannot recur');
    });
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
    /// This used to assert the opposite — that a missing digest falls back to
    /// size-only. That IS what the code did, and it is the wrong policy for a
    /// gate whose job is "never run what you could not verify": size alone only
    /// catches truncation, so a same-length replacement passed.
    ///
    /// GitHub populates `assets[].digest` for every asset today (checked
    /// 2026-08-05 against v0.13.13), so this path should never fire in
    /// practice. That is precisely why it has to refuse rather than degrade —
    /// otherwise the day it stops being populated, nothing announces it.
    test('refuses when the release carries no digest — never size-only', () {
      expect(installerMatches(bytes, size: 3), isFalse);
      expect(installerMatches(bytes), isFalse);
    });
  });
}
