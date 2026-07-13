// WebSocket client for a single collaborative document room (presence +
// accepted-update sequence) plus the socket-URI helper. Extracted from
// main.dart (2026-07).
import 'dart:async';
import 'dart:convert';

import 'package:web_socket_channel/web_socket_channel.dart';

import 'models.dart';

typedef RemoteSeqCallback = void Function(String documentId, int serverSeq);
typedef PresenceCallback = void Function(List<PresenceUser> users);

/// WebSocket client for a single document room.
///
/// Receives the server's accepted-update sequence (so the shell can pull the
/// latest snapshot) and tracks presence of other collaborators. Local edits
/// continue to flow over REST, which the backend broadcasts here.
class DocumentSyncClient {
  DocumentSyncClient({
    required this.documentId,
    required this.uri,
    required this.selfName,
    required this.onRemoteSeq,
    required this.onPresence,
  });

  final String documentId;
  final Uri uri;
  final String selfName;
  final RemoteSeqCallback onRemoteSeq;
  final PresenceCallback onPresence;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _subscription;
  String? _connectionId;
  final Map<String, PresenceUser> _presence = {};
  bool _disposed = false;

  void connect() {
    final channel = WebSocketChannel.connect(uri);
    _channel = channel;
    _subscription = channel.stream.listen(
      _onMessage,
      onError: (_) {},
      onDone: () {},
      cancelOnError: false,
    );
  }

  void _onMessage(dynamic raw) {
    if (raw is! String) {
      return;
    }

    final Map<String, dynamic> message;
    try {
      message = jsonDecode(raw) as Map<String, dynamic>;
    } catch (_) {
      return;
    }

    switch (message['type']) {
      case 'document.bootstrap':
        _connectionId = message['connection_id'] as String?;
        _sendPresence();
      case 'document.update.accepted':
        final seq = message['server_seq'];
        if (seq is int) {
          onRemoteSeq(documentId, seq);
        }
      case 'presence.state':
        _presence.clear();
        final list = message['presences'];
        if (list is List<dynamic>) {
          for (final entry in list) {
            if (entry is Map<String, dynamic>) {
              _upsertPresence(entry);
            }
          }
        }
        _emitPresence();
      case 'presence.update':
        _upsertPresence(message);
        _emitPresence();
      case 'presence.leave':
        final connectionId = message['connection_id'];
        if (connectionId is String) {
          _presence.remove(connectionId);
        }
        _emitPresence();
    }
  }

  void _upsertPresence(Map<String, dynamic> message) {
    final connectionId = message['connection_id'];
    final userId = message['user_id'];
    if (connectionId is! String || userId is! String) {
      return;
    }

    var name = userId;
    String? cursorBlock;
    int? cursorOffset;
    final data = message['data'];
    if (data is Map<String, dynamic>) {
      if (data['name'] is String) name = data['name'] as String;
      final cursor = data['cursor'];
      if (cursor is Map &&
          cursor['block'] is String &&
          cursor['offset'] is int) {
        cursorBlock = cursor['block'] as String;
        cursorOffset = cursor['offset'] as int;
      }
    }

    _presence[connectionId] = PresenceUser(
      connectionId: connectionId,
      userId: userId,
      name: name,
      cursorBlockId: cursorBlock,
      cursorOffset: cursorOffset,
    );
  }

  void _emitPresence() {
    if (_disposed) {
      return;
    }
    final others = _presence.values
        .where((user) => user.connectionId != _connectionId)
        .toList();
    onPresence(others);
  }

  Map<String, dynamic>? _cursor;

  /// Broadcast the local caret (block id + offset) as awareness; null clears it.
  void sendCursor(String? blockId, int? offset) {
    _cursor = (blockId != null && offset != null)
        ? {'block': blockId, 'offset': offset}
        : null;
    _sendPresence();
  }

  void _sendPresence() {
    _channel?.sink.add(
      jsonEncode({
        'type': 'presence.update',
        'payload': {'name': selfName, if (_cursor != null) 'cursor': _cursor},
      }),
    );
  }

  void dispose() {
    _disposed = true;
    _subscription?.cancel();
    _channel?.sink.close();
    _channel = null;
  }
}

Uri documentSocketUri(
  Uri base,
  String workspaceId,
  String documentId,
  String token,
) {
  final scheme = base.scheme == 'https' ? 'wss' : 'ws';
  return base.replace(
    scheme: scheme,
    path: '/ws/workspaces/$workspaceId/documents/$documentId',
    queryParameters: {'token': token},
  );
}
