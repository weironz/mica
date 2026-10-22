// Every path that ends the process must persist local state first — the
// updater's "更新并重启" used to `exit(0)` straight past it (2026-10-21 review
// U4), dropping the last ≤400ms of typing and the local/cloud durability flush.
import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/window_setup_desktop.dart';

void main() {
  tearDown(() => appExitFlush = null);

  test('flushBeforeExit runs the registered flush and waits for it', () async {
    var done = false;
    appExitFlush = () async {
      await Future<void>.delayed(const Duration(milliseconds: 50));
      done = true;
    };
    await flushBeforeExit();
    expect(done, isTrue);
  });

  test('a wedged or failing flush cannot hold the exit hostage', () async {
    appExitFlush = () => Completer<void>().future; // never completes
    final watch = Stopwatch()..start();
    await flushBeforeExit();
    expect(watch.elapsed, lessThan(const Duration(seconds: 3)));

    appExitFlush = () async => throw StateError('store gone');
    await flushBeforeExit(); // must not throw
  });
}
