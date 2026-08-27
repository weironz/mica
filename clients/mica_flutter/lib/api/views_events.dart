// "The sidebar tree changed" channel — the client half of
// `/ws/workspaces/{id}/views` (server: `ws.rs::views_socket`, fed by a Postgres
// trigger via LISTEN/NOTIFY, migration 0025).
//
// Before this, a page created or deleted by anyone who is not this client —
// MCP, the CLI, another device — sat invisible until the user pressed the
// sidebar's refresh button. The tree data was always cheap to re-ask for
// (`listViewsIfChanged` answers 304 off an ETag); what was missing was the
// SIGNAL, and this socket is only that. It carries no tree data downstream and
// nothing at all upstream.
import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import 'ws_connect.dart';

/// The socket URI for one workspace's tree-change pings.
///
/// Shaped like `documentSocketUri` (same `?token=` seam — `connectAuthedSocket`
/// lifts it into an Authorization header on IO), minus the `v` protocol
/// parameter: the sync protocol's envelope negotiation has nothing to negotiate
/// on a channel whose one message kind is its whole contract.
Uri viewsSocketUri(Uri base, String workspaceId, String token) {
  final scheme = base.scheme == 'https' ? 'wss' : 'ws';
  return base.replace(
    scheme: scheme,
    path: '/ws/workspaces/$workspaceId/views',
    queryParameters: {'token': token},
  );
}

/// Holds the socket open for as long as a workspace's sidebar is showing, and
/// turns whatever arrives on it into debounced [onChanged] calls.
///
/// Debounced because one user gesture is often several rows server-side (an
/// import, a batch move) and Postgres only dedups within one transaction —
/// refetching the tree once per burst is the whole point of the bell-not-data
/// design. Missed bells are never a correctness problem: [onChanged] re-asks
/// for the CURRENT tree, so the last call in any burst sees everything.
///
/// Reconnects on its own (capped exponential backoff, same policy as
/// `CloudSyncSession`), and fires [onChanged] once after every REconnect: bells
/// that rang during the gap are gone, and the refetch is how they are found
/// again. The first connect deliberately does not fire — the caller just
/// fetched the tree it is showing.
class ViewsEventsChannel {
  ViewsEventsChannel({
    required this.uri,
    required this.onChanged,
    this.debounce = const Duration(milliseconds: 400),
  });

  /// Built per connection ATTEMPT, not held: the URI carries the access token,
  /// which lapses in an hour, while a sidebar stays open all day. The host
  /// renews inside this callback (`_ensureFreshSession`), exactly as the
  /// document sockets do — see `CloudSyncSession.uri` for the incident that
  /// shape comes from.
  final Future<Uri> Function() uri;

  /// "Something about this workspace's tree changed — re-ask for it."
  final void Function() onChanged;

  final Duration debounce;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _messages;
  Timer? _reconnectTimer;
  Timer? _debounceTimer;
  int _attempt = 0;
  bool _connecting = false;
  bool _everConnected = false;
  bool _disposed = false;

  void connect() {
    if (_disposed || _connecting || _channel != null) return;
    _connecting = true;
    unawaited(_open());
  }

  Future<void> _open() async {
    try {
      final target = await uri();
      if (_disposed) return;
      final channel = connectAuthedSocket(target);
      _channel = channel;
      _messages = channel.stream.listen(
        // Every message means the same thing, so the payload is not even
        // parsed: an unknown future message kind degrades to a refetch that
        // finds nothing new, not to a crash in a listener.
        (_) => _ping(),
        onDone: _dropAndReconnect,
        onError: (Object _) => _dropAndReconnect(),
      );
      _attempt = 0;
      if (_everConnected) _ping();
      _everConnected = true;
    } catch (_) {
      _dropAndReconnect();
    } finally {
      _connecting = false;
    }
  }

  void _ping() {
    if (_disposed) return;
    _debounceTimer ??= Timer(debounce, () {
      _debounceTimer = null;
      if (!_disposed) onChanged();
    });
  }

  void _dropAndReconnect() {
    _messages?.cancel();
    _messages = null;
    _channel?.sink.close();
    _channel = null;
    if (_disposed || _reconnectTimer != null) return;
    // 1s, 2s, 4s … capped at 30s: a dead server costs a ping every half
    // minute, and a restarted one is noticed within seconds.
    final seconds = 1 << _attempt.clamp(0, 5);
    _attempt += 1;
    _reconnectTimer = Timer(Duration(seconds: seconds > 30 ? 30 : seconds), () {
      _reconnectTimer = null;
      connect();
    });
  }

  /// Fire the debounced change path as if a message arrived — the seam the
  /// debounce tests drive, since a real socket needs a real server.
  @visibleForTesting
  void debugPing() => _ping();

  void dispose() {
    _disposed = true;
    _reconnectTimer?.cancel();
    _debounceTimer?.cancel();
    _messages?.cancel();
    _channel?.sink.close();
    _channel = null;
  }
}
