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
import 'cloud_doc_store.dart';

export 'cloud_doc_store.dart';

typedef DocOp = Map<String, dynamic>;

/// A local diff awaiting the server's ack, tagged with a client id the server
/// echoes in `sync.ack`. Parity with the desktop session.
class _Pending {
  _Pending(this.id, this.bytes);
  final String id;
  final Uint8List bytes;
  bool sent = false;
}

class CloudSyncSession {
  CloudSyncSession({
    required this.uri,
    required this.clientId,
    required this.onReady,
    required this.onRemoteBlocks,
    this.onFault,
    this.onServerConnected,
    this.restoreUnacked,
    this.onPersistUnacked,
    this.persistence,
  });

  final Uri uri;

  /// Accepted for parity with the desktop session. yjs assigns a per-session
  /// actor id; the web client has no on-device store to pin one to yet, and a
  /// fresh actor per page load is CRDT-correct (convergence still holds).
  final BigInt clientId;

  final void Function(String rootBlockId, List<Map<String, dynamic>> blocks)
  onReady;
  final void Function(List<Map<String, dynamic>> blocks) onRemoteBlocks;

  /// Integrity-fault hook — parity with the desktop session (red line #1).
  final void Function(String reason, int count)? onFault;

  /// Accepted for parity with the desktop session; unused on web (P1c offline nav
  /// is desktop-only — web has no on-device store, so it never enters the
  /// offline-nav fallback that this signal recovers from).
  final void Function()? onServerConnected;

  /// Crash-recovery parity (C1): unacked diffs restored at startup + a persist
  /// callback fired when the queue changes.
  final List<Uint8List>? restoreUnacked;
  final void Function(List<Uint8List> unacked)? onPersistUnacked;

  /// Accepted for parity with the desktop session; unused on web (CanvasKit has
  /// no on-device store yet — web cloud stays online, P2 Phase 1). Pass null.
  final CloudDocStore? persistence;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _sub;
  MicaYDoc? _doc;
  String _rootBlockId = '';
  int _cursor = 0;
  bool _ready = false;
  bool _disposed = false;
  int _faultCount = 0;
  static const int _maxAutoReheal = 3;
  final List<_Pending> _unacked = [];
  int _pushSeq = 0;
  bool _restored = false;
  Timer? _persistTimer;

  /// Auto-reconnect with capped backoff — parity with the desktop session.
  Timer? _reconnectTimer;
  int _reconnectAttempts = 0;

  String get rootBlockId => _rootBlockId;
  bool get isReady => _ready;

  // §6 migration parity (desktop-only feature; present here so main.dart compiles
  // for web — the web build never drives a migration since local offline is
  // native-only).
  final Completer<void> _readyCompleter = Completer<void>();
  Future<void> get ready => _readyCompleter.future;

  /// Best-effort flush + report whether the unacked queue is empty (B4 parity).
  Future<bool> drainOutbox({Duration timeout = const Duration(seconds: 15)}) async {
    final deadline = DateTime.now().add(timeout);
    while (!_disposed && DateTime.now().isBefore(deadline)) {
      if (_channel != null && _ready) _flushUnacked();
      if (_unacked.isEmpty) return true;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    return _unacked.isEmpty;
  }

  /// Graceful teardown (C2 parity): flush before closing.
  Future<bool> drainAndDispose({
    Duration timeout = const Duration(seconds: 4),
  }) async {
    final drained = await drainOutbox(timeout: timeout);
    dispose();
    return drained;
  }

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
    _reconnectTimer?.cancel();
    _reconnectTimer = null;
    _sub?.cancel();
    _restoreUnackedOnce();
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
      _flushUnacked(resendAll: true);
    }
  }

  void _restoreUnackedOnce() {
    if (_restored) return;
    _restored = true;
    final restore = restoreUnacked;
    if (restore == null || restore.isEmpty) return;
    for (final bytes in restore) {
      _unacked.add(_Pending('${_pushSeq++}', bytes));
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
    // A valid frame means the link is live — reset the reconnect backoff.
    _reconnectAttempts = 0;
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
          final ok = existing.applyUpdate(base64.decode(b64));
          if (!ok) {
            _onIntegrityFault('bad_base');
            return;
          }
          _cursor = baseRid;
          if (!_disposed) onRemoteBlocks(childBlocks());
        }
        // Recovery worked — reset the consecutive-fault count (B3 parity).
        _faultCount = 0;
        _send({
          'type': 'sync.pull',
          'payload': {'since_rid': _cursor},
        });
        _flushUnacked(resendAll: true);
      case 'sync.updates':
        final ups = m['updates'];
        final before = _cursor;
        var changed = false;
        if (ups is List) {
          for (final u in ups) {
            if (u is Map && _applyRemote(u.cast<String, dynamic>())) {
              changed = true;
            }
          }
        }
        if (changed && !_disposed) onRemoteBlocks(childBlocks());
        // B2 (verified catch-up): keep pulling until the stream after our cursor
        // drains, so a server-capped batch can't silently truncate. Mirrors the
        // desktop session.
        if (ups is List &&
            ups.isNotEmpty &&
            _cursor > before &&
            _channel != null &&
            !_disposed) {
          _send({
            'type': 'sync.pull',
            'payload': {'since_rid': _cursor},
          });
        }
      case 'sync.update':
        if (_applyRemote(m) && !_disposed) onRemoteBlocks(childBlocks());
      case 'sync.ack':
        final ackId = m['ack_id'];
        if (ackId is String) {
          final before = _unacked.length;
          _unacked.removeWhere((p) => p.id == ackId);
          if (_unacked.length != before) _persistSoon();
        }
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
    if (!ok) {
      // Red line #1: don't advance the cursor past an update we couldn't apply
      // (silent divergence). Self-heal via a capped re-bootstrap; surface the
      // fault. Mirrors the desktop session.
      _onIntegrityFault('bad_remote_update');
      return false;
    }
    final rid = (u['rid'] as num?)?.toInt();
    if (rid != null && rid > _cursor) _cursor = rid;
    return true;
  }

  void _onIntegrityFault(String reason) {
    _faultCount++;
    onFault?.call(reason, _faultCount);
    if (_channel != null && !_disposed && _faultCount <= _maxAutoReheal) {
      _send({'type': 'sync.bootstrap'});
    }
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
    _enqueue(diff);
  }

  void _enqueue(Uint8List diff) {
    final p = _Pending('${_pushSeq++}', diff);
    _unacked.add(p);
    _persistSoon();
    if (_channel != null && _ready) _sendPush(p);
  }

  void _flushUnacked({bool resendAll = false}) {
    if (_channel == null || !_ready) return;
    for (final p in _unacked) {
      if (resendAll || !p.sent) _sendPush(p);
    }
  }

  void _sendPush(_Pending p) {
    p.sent = true;
    _send({
      'type': 'sync.push',
      'id': p.id,
      'payload': {'update': base64.encode(p.bytes)},
    });
  }

  void _persistSoon() {
    if (onPersistUnacked == null) return;
    _persistTimer?.cancel();
    _persistTimer = Timer(const Duration(milliseconds: 300), _persistNow);
  }

  void _persistNow() {
    _persistTimer?.cancel();
    _persistTimer = null;
    onPersistUnacked?.call([for (final p in _unacked) p.bytes]);
  }

  void _send(Map<String, dynamic> message) {
    _channel?.sink.add(jsonEncode(message));
  }

  void _onDone() {
    _channel = null;
    _scheduleReconnect();
  }

  /// Reconnect with capped exponential backoff (0.5s → 30s) — parity with io.
  void _scheduleReconnect() {
    if (_disposed || _channel != null || _reconnectTimer != null) return;
    final shift = _reconnectAttempts.clamp(0, 6);
    _reconnectAttempts++;
    final ms = (500 << shift).clamp(500, 30000).toInt();
    _reconnectTimer = Timer(Duration(milliseconds: ms), () {
      _reconnectTimer = null;
      if (_disposed || _channel != null) return;
      connect();
    });
  }

  void dispose() {
    // Best-effort flush + synchronous persist before closing (C1/C2 parity).
    if (!_disposed && _channel != null && _ready) {
      _flushUnacked();
    }
    _persistNow();
    _disposed = true;
    _reconnectTimer?.cancel();
    _sub?.cancel();
    _channel?.sink.close();
    _channel = null;
  }
}
