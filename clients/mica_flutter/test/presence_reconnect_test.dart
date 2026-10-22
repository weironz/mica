import 'dart:async';
import 'dart:convert';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/models.dart';
import 'package:mica_flutter/api/sync_client.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

class _FakeChannel implements WebSocketChannel {
  final incoming = StreamController<dynamic>();
  final sent = <String>[];

  @override
  Future<void> get ready => Future.value();

  @override
  Stream<dynamic> get stream => incoming.stream;

  @override
  WebSocketSink get sink => _FakeSink(sent);

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _FakeSink implements WebSocketSink {
  _FakeSink(this.sent);
  final List<String> sent;

  @override
  void add(dynamic data) => sent.add(data as String);

  @override
  Future<void> close([int? closeCode, String? closeReason]) async {}

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// Issue #53: the presence socket used to stay dead after one drop, and its
/// URI (with the token in it) was fixed at construction.
void main() {
  test('a dropped presence socket reconnects with a freshly built URI', () {
    fakeAsync((async) {
      final channels = <_FakeChannel>[];
      final uris = <int>[];
      var rosters = <List<PresenceUser>>[];
      final client = DocumentSyncClient(
        documentId: 'd1',
        uri: () async {
          uris.add(uris.length);
          return Uri.parse('ws://h/doc?token=t${uris.length}');
        },
        selfName: 'me',
        onRemoteSeq: (_, _) {},
        onPresence: rosters.add,
        connectSocket: (_) {
          final c = _FakeChannel();
          channels.add(c);
          return c;
        },
      );
      client.connect();
      async.flushMicrotasks();
      expect(channels, hasLength(1));

      channels[0].incoming.add(jsonEncode({
        'type': 'presence.state',
        'presences': [
          {'connection_id': 'other', 'user_id': 'u2', 'data': {'name': 'Ada'}},
        ],
      }));
      async.flushMicrotasks();
      expect(rosters.last.map((u) => u.name), ['Ada']);

      // The connection drops.
      channels[0].incoming.close();
      async.flushMicrotasks();
      expect(rosters.last, isEmpty, reason: 'stale avatars are cleared');

      async.elapse(const Duration(seconds: 2));
      expect(channels, hasLength(2), reason: 'reconnected after the backoff');
      expect(uris, hasLength(2), reason: 'the URI (token) is rebuilt per attempt');

      // The new connection announces us again on bootstrap.
      channels[1].incoming.add(jsonEncode({'type': 'document.bootstrap', 'connection_id': 'c2'}));
      async.flushMicrotasks();
      expect(channels[1].sent.single, contains('presence.update'));

      client.dispose();
      async.elapse(const Duration(minutes: 1));
      expect(channels, hasLength(2), reason: 'disposed: no further attempts');
    });
  });
}
