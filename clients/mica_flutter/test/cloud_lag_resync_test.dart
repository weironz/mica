// Regression: a server "client_out_of_date" lag notice must trigger a resync.
//
// When the per-document broadcast channel overflows, the server sends
// {type:'error', code:'client_out_of_date'} with NO ack_id. The session's
// 'error' handler only recognised a push rejection (a String ack_id), so this
// notice fell through and was dropped — leaving the replica silently behind
// (red line #1). It now catches up with a fresh pull/bootstrap.
@TestOn('vm')
library;

import 'dart:convert';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/cloud_sync_session.dart';

CloudSyncSession _session() => CloudSyncSession(
      uri: () async => Uri.parse('ws://example.invalid/doc'),
      clientId: BigInt.from(1),
      onReady: (_, _) {},
      onRemoteBlocks: (_) {},
    );

void main() {
  test('a client_out_of_date lag notice triggers a resync', () {
    final s = _session();
    expect(s.debugLagResyncCount, 0);

    s.debugHandleFrame(jsonEncode({
      'type': 'error',
      'code': 'client_out_of_date',
      'message': 'missed updates; reload the document to resync',
    }));

    expect(s.debugLagResyncCount, 1,
        reason: 'the no-ack_id lag notice must trigger a catch-up resync');
  });

  test('a plain push rejection is NOT treated as a lag notice', () {
    final s = _session();
    s.debugHandleFrame(jsonEncode({'type': 'error', 'ack_id': '7'}));
    expect(s.debugLagResyncCount, 0,
        reason: 'a String ack_id is a push rejection, not a stream lag');
  });

  test('a burst of lag notices debounces to a single resync', () {
    final s = _session();
    for (var i = 0; i < 5; i++) {
      s.debugHandleFrame(jsonEncode({
        'type': 'error',
        'code': 'client_out_of_date',
      }));
    }
    expect(s.debugLagResyncCount, 1,
        reason: 'closely-spaced notices cost one catch-up, not a storm');
    s.dispose();
  });

  // A notice inside the debounce window may report a drop that happened AFTER
  // the first catch-up was answered; ignoring it lost that drop for good.
  test('a notice inside the debounce window gets one trailing resync', () {
    fakeAsync((async) {
      final s = _session();
      for (var i = 0; i < 3; i++) {
        s.debugHandleFrame(jsonEncode({
          'type': 'error',
          'code': 'client_out_of_date',
        }));
      }
      expect(s.debugLagResyncCount, 1);
      async.elapse(const Duration(milliseconds: 1100));
      expect(s.debugLagResyncCount, 2,
          reason: 'the in-window notices are covered by one trailing resync');
      s.dispose();
    });
  });
}
