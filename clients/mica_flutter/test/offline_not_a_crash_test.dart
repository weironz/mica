// Being unable to reach the server is a state, not a crash.
//
// `WebSocketChannel.connect` reports a failed connection twice: on the stream
// (which the session listens to, and turns into the offline badge plus a backoff
// reconnect) and on `ready`. Nobody observed `ready`, so every offline moment
// became an uncaught zone error and wrote a `WebSocketChannelException` into
// `crash.log` — the file where real faults are supposed to stand out.
//
// Found by running the desktop app with the backend down; no test would have
// noticed, because nothing was broken except the signal.
@TestOn('vm')
library;

import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';

void main() {
  /// A port nothing is listening on: bound to learn a free one, then released.
  Future<int> deadPort() async {
    final probe = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final port = probe.port;
    await probe.close();
    return port;
  }

  /// Runs a session against [port] for long enough to fail, collecting both what
  /// escaped to the zone and what the session reported.
  Future<({List<Object> uncaught, List<SyncPhase> phases})> attempt(
    int port,
  ) async {
    final uncaught = <Object>[];
    final phases = <SyncPhase>[];
    await runZonedGuarded(() async {
      final session = CloudSyncSession(
        uri: () async => Uri.parse('ws://127.0.0.1:$port/ws/doc'),
        clientId: BigInt.one,
        onReady: (_, _) {},
        onRemoteBlocks: (_) {},
        onSyncPhase: phases.add,
      );
      session.connect();
      // Long enough for the connect to fail and for an unobserved error to
      // reach the zone.
      await Future<void>.delayed(const Duration(milliseconds: 400));
      session.dispose();
    }, (error, stack) => uncaught.add(error));
    return (uncaught: uncaught, phases: phases);
  }

  test('a server that cannot be reached raises no uncaught error', () async {
    final r = await attempt(await deadPort());
    expect(
      r.uncaught,
      isEmpty,
      reason: 'offline must not look like a crash: ${r.uncaught}',
    );
  });

  test('the failure is handled, not swallowed: it retries', () async {
    // The other half of the fix. Dropping the duplicate error must not drop the
    // signal — and the observable proof is that a retry actually arrives once
    // something starts listening. (The sync badge is NOT the right assertion
    // here: the session starts offline, so a failed first connect is no change
    // of phase and `_updateSyncPhase` deliberately emits nothing.)
    final port = await deadPort();
    final uncaught = <Object>[];
    final connected = Completer<void>();

    await runZonedGuarded(() async {
      final session = CloudSyncSession(
        uri: () async => Uri.parse('ws://127.0.0.1:$port/ws/doc'),
        clientId: BigInt.one,
        onReady: (_, _) {},
        onRemoteBlocks: (_) {},
      );
      session.connect();
      await Future<void>.delayed(const Duration(milliseconds: 200));

      // Now the server "comes back".
      final server = await ServerSocket.bind(
        InternetAddress.loopbackIPv4,
        port,
      );
      server.listen((socket) {
        if (!connected.isCompleted) connected.complete();
        socket.destroy();
      });

      await connected.future.timeout(
        const Duration(seconds: 5),
        onTimeout: () => throw StateError('no reconnect attempt arrived'),
      );
      session.dispose();
      await server.close();
    }, (error, stack) => uncaught.add(error));

    expect(connected.isCompleted, isTrue);
    expect(
      uncaught,
      isEmpty,
      reason: 'still no crash while retrying: $uncaught',
    );
  });
}
