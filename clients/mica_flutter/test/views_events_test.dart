// The debounce contract of the tree-change channel. The SOCKET half needs a
// real server and is exercised by the server-side integration check (a WS
// client watching a workspace while a page is created over REST); what can go
// quietly wrong client-side is the burst arithmetic, and that is pinned here
// through the same `_ping` path a real message takes (`debugPing`).
import 'dart:async';

import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/views_events.dart';
import 'package:mica_flutter/swallowed.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

class _RejectedChannel implements WebSocketChannel {
  final _ready = Completer<void>();
  final _stream = StreamController<dynamic>();

  @override
  Future<void> get ready => _ready.future;

  @override
  Stream<dynamic> get stream => _stream.stream;

  @override
  WebSocketSink get sink => _TestSink();

  void reject() => _ready.completeError(StateError('handshake refused'));

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _TestSink implements WebSocketSink {
  @override
  Future<void> close([int? closeCode, String? closeReason]) async {}

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  group('viewsSocketUri', () {
    test('ws for http, wss for https, token in the query seam', () {
      final ws = viewsSocketUri(Uri.parse('http://h:8080/api'), 'w1', 't');
      expect(ws.scheme, 'ws');
      expect(ws.path, '/ws/workspaces/w1/views');
      expect(ws.queryParameters['token'], 't');
      expect(
        viewsSocketUri(Uri.parse('https://h/api'), 'w1', 't').scheme,
        'wss',
      );
    });
  });

  group('ViewsEventsChannel debounce', () {
    test('a burst of pings becomes ONE onChanged', () {
      fakeAsync((async) {
        var calls = 0;
        final channel = ViewsEventsChannel(
          uri: () async => Uri.parse('ws://unused'),
          onChanged: () => calls++,
        );
        // An import: many rows, many bells, close together.
        channel.debugPing();
        async.elapse(const Duration(milliseconds: 100));
        channel.debugPing();
        async.elapse(const Duration(milliseconds: 100));
        channel.debugPing();
        expect(calls, 0, reason: 'nothing fires inside the window');
        async.elapse(const Duration(milliseconds: 400));
        expect(calls, 1, reason: 'the burst collapses to one refetch');
        channel.dispose();
      });
    });

    test('a ping after the window fires again', () {
      fakeAsync((async) {
        var calls = 0;
        final channel = ViewsEventsChannel(
          uri: () async => Uri.parse('ws://unused'),
          onChanged: () => calls++,
        );
        channel.debugPing();
        async.elapse(const Duration(milliseconds: 500));
        channel.debugPing();
        async.elapse(const Duration(milliseconds: 500));
        expect(calls, 2);
        channel.dispose();
      });
    });

    test('dispose inside the window suppresses the pending call', () {
      // The shell disposes this channel on workspace switch; a refetch that
      // fired AFTER that would write the OLD workspace's tree into state the
      // new workspace is about to own.
      fakeAsync((async) {
        var calls = 0;
        final channel = ViewsEventsChannel(
          uri: () async => Uri.parse('ws://unused'),
          onChanged: () => calls++,
        );
        channel.debugPing();
        channel.dispose();
        async.elapse(const Duration(seconds: 1));
        expect(calls, 0);
      });
    });
  });

  test('rejected handshake is observed and enters reconnect backoff', () async {
    resetSwallowed();
    final socket = _RejectedChannel();
    var connects = 0;
    var changed = 0;
    final channel = ViewsEventsChannel(
      uri: () async => Uri.parse('ws://unused'),
      onChanged: () => changed++,
      connectSocket: (_) {
        connects++;
        return socket;
      },
    );
    addTearDown(channel.dispose);
    channel.connect();
    await Future<void>.delayed(Duration.zero);
    expect(connects, 1);
    socket.reject();
    await Future<void>.delayed(Duration.zero);
    expect(swallowedCounts()['views_ws_ready'], 1);
    // `ready` is the only failing path in this fake; the stream stays open.
    // A retry therefore proves the failure drives the normal reconnect path.
    await Future<void>.delayed(const Duration(milliseconds: 1100));
    expect(connects, 2);
    expect(changed, 0, reason: 'a failed handshake is not a reconnect');
  });
}
