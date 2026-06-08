// P2-M4 W3: the WEB cloud editing session — same WS sync protocol as the desktop
// [CloudSyncSession] (cloud_sync_io.dart), but the CRDT replica is a JS `yjs`
// document ([MicaYDoc]) instead of the Rust `yrs` FFI. yjs and yrs are
// wire-compatible (verified W1/W2), so a web client and a desktop client
// editing the same document converge with no translation.
//
// Web-only (imports `dart:js_interop` via mica_ydoc); the facade `cloud_sync.dart`
// picks this on web and the FFI version on desktop.
import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:web_socket_channel/web_socket_channel.dart';

import '../web/mica_ydoc.dart';

typedef DocOp = Map<String, dynamic>;

class CloudSyncSession {
  CloudSyncSession({
    required this.uri,
    required this.clientId,
    required this.onReady,
    required this.onRemoteBlocks,
  });

  final Uri uri;

  /// Accepted for parity with the desktop session. yjs assigns a per-session
  /// actor id; the web client has no on-device store to pin one to yet, and a
  /// fresh actor per page load is CRDT-correct (convergence still holds).
  final BigInt clientId;

  final void Function(String rootBlockId, List<Map<String, dynamic>> blocks)
  onReady;
  final void Function(List<Map<String, dynamic>> blocks) onRemoteBlocks;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _sub;
  MicaYDoc? _doc;
  String _rootBlockId = '';
  int _cursor = 0;
  bool _ready = false;
  bool _disposed = false;
  final List<Uint8List> _outbox = [];

  String get rootBlockId => _rootBlockId;
  bool get isReady => _ready;

  List<Map<String, dynamic>> allBlocks() => _doc?.toBlocks() ?? const [];

  List<Map<String, dynamic>> childBlocks() {
    final doc = _doc;
    if (doc == null) return const [];
    final byId = {for (final b in doc.toBlocks()) b['id'] as String: b};
    final root = byId[_rootBlockId];
    if (root == null) return const [];
    final children = (root['children'] as List?)?.cast<String>() ?? const [];
    return [
      for (final id in children)
        if (byId[id] != null) byId[id]!,
    ];
  }

  void connect() {
    if (_disposed) return;
    final channel = WebSocketChannel.connect(uri);
    _channel = channel;
    _sub = channel.stream.listen(
      _onMessage,
      onError: (_) {},
      onDone: _onDone,
      cancelOnError: false,
    );
    if (_doc == null) {
      _send({'type': 'sync.bootstrap'});
    } else {
      _send({
        'type': 'sync.pull',
        'payload': {'since_rid': _cursor},
      });
      _flushOutbox();
    }
  }

  void _onMessage(dynamic raw) {
    if (raw is! String) return;
    Map<String, dynamic> m;
    try {
      m = jsonDecode(raw) as Map<String, dynamic>;
    } catch (_) {
      return;
    }
    switch (m['type']) {
      case 'sync.base':
        final b64 = m['base'];
        if (b64 is! String) return;
        final baseRid = (m['base_rid'] as num?)?.toInt() ?? 0;
        final existing = _doc;
        if (existing == null) {
          final doc = MicaYDoc.fromState(base64.decode(b64));
          _doc = doc;
          _rootBlockId = doc.rootBlockId();
          _cursor = baseRid;
          _ready = true;
          onReady(_rootBlockId, childBlocks());
        } else if (baseRid > _cursor) {
          // Re-bootstrap after stream pruning: merge the base, keep local edits.
          existing.applyUpdate(base64.decode(b64));
          _cursor = baseRid;
          if (!_disposed) onRemoteBlocks(childBlocks());
        }
        _send({
          'type': 'sync.pull',
          'payload': {'since_rid': _cursor},
        });
        _flushOutbox();
      case 'sync.updates':
        final ups = m['updates'];
        var changed = false;
        if (ups is List) {
          for (final u in ups) {
            if (u is Map && _applyRemote(u.cast<String, dynamic>())) {
              changed = true;
            }
          }
        }
        if (changed && !_disposed) onRemoteBlocks(childBlocks());
      case 'sync.update':
        if (_applyRemote(m) && !_disposed) onRemoteBlocks(childBlocks());
      case 'sync.ack':
        final rid = (m['rid'] as num?)?.toInt();
        if (rid != null && rid > _cursor) _cursor = rid;
    }
  }

  bool _applyRemote(Map<String, dynamic> u) {
    final doc = _doc;
    if (doc == null) return false;
    final b64 = u['update'];
    if (b64 is! String) return false;
    final ok = doc.applyUpdate(base64.decode(b64));
    final rid = (u['rid'] as num?)?.toInt();
    if (rid != null && rid > _cursor) _cursor = rid;
    return ok;
  }

  void applyLocalOps(List<DocOp> ops) {
    final doc = _doc;
    if (doc == null) return;
    final sv = doc.stateVector();
    for (final op in ops) {
      doc.applyOp(op);
    }
    final diff = doc.encodeDiffSince(sv);
    if (diff.isEmpty) return;
    _pushOrQueue(diff);
  }

  void _pushOrQueue(Uint8List diff) {
    if (_channel != null && _ready) {
      _send({
        'type': 'sync.push',
        'payload': {'update': base64.encode(diff)},
      });
    } else {
      _outbox.add(diff);
    }
  }

  void _flushOutbox() {
    if (_channel == null || !_ready) return;
    final pending = List<Uint8List>.from(_outbox);
    _outbox.clear();
    for (final diff in pending) {
      _send({
        'type': 'sync.push',
        'payload': {'update': base64.encode(diff)},
      });
    }
  }

  void _send(Map<String, dynamic> message) {
    _channel?.sink.add(jsonEncode(message));
  }

  void _onDone() {
    _channel = null;
  }

  void dispose() {
    _disposed = true;
    _sub?.cancel();
    _channel?.sink.close();
    _channel = null;
  }
}
