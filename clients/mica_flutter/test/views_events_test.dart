// The debounce contract of the tree-change channel. The SOCKET half needs a
// real server and is exercised by the server-side integration check (a WS
// client watching a workspace while a page is created over REST); what can go
// quietly wrong client-side is the burst arithmetic, and that is pinned here
// through the same `_ping` path a real message takes (`debugPing`).
import 'package:fake_async/fake_async.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/views_events.dart';

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
}
