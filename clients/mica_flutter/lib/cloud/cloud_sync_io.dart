// P2-M4.5: a live cloud editing session for one document, backed by a yrs CRDT
// replica.
//
// It owns a [MicaDocument] (the device's replica, pinned to the device's stable
// yrs client id), speaks the WS sync protocol added in M4.4
// (sync.bootstrap/pull/push + sync.update), pushes local editor ops as yrs
// diffs, and merges remote updates — firing [onRemoteBlocks] so the editor can
// reconcile. CRDT merge makes concurrent edits converge with no central locking.
//
// Not imported on web (depends on the native FFI); callers guard with `!kIsWeb`.
import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:web_socket_channel/web_socket_channel.dart';

import '../local/doc_ops.dart';
import '../src/rust/api/document.dart';

class CloudSyncSession {
  CloudSyncSession({
    required this.uri,
    required this.clientId,
    required this.onReady,
    required this.onRemoteBlocks,
  });

  /// The document WebSocket URI (already carrying the auth token).
  final Uri uri;

  /// This device's stable yrs client id (from the local store identity) — so all
  /// of a device's edits share one CRDT actor across sessions.
  final BigInt clientId;

  /// Fired once after bootstrap with the root block id + the editor's nodes.
  final void Function(String rootBlockId, List<Map<String, dynamic>> blocks)
  onReady;

  /// Fired after remote updates are merged, with the refreshed editor nodes.
  final void Function(List<Map<String, dynamic>> blocks) onRemoteBlocks;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _sub;
  MicaDocument? _doc;
  final DocOpMirror _mirror = DocOpMirror();
  String _rootBlockId = '';

  /// Highest stream rid this replica has applied (per-document cursor).
  int _cursor = 0;
  bool _ready = false;
  bool _disposed = false;

  /// Completes when the session first becomes ready (cold bootstrap done). Lets
  /// headless callers (e.g. the §6 migrator) await a usable replica.
  final Completer<void> _readyCompleter = Completer<void>();

  /// Push/ack accounting so [drainOutbox] knows when the server has folded every
  /// diff we sent (each `sync.push` is answered by one `sync.ack` carrying a rid).
  int _pushCount = 0;
  int _ackCount = 0;

  /// Local yrs diffs produced before the socket was ready / while offline,
  /// flushed to the cloud on (re)connect. Survives reconnects since [_doc] is
  /// kept across them (never rebuilt once it holds unpushed edits).
  final List<Uint8List> _outbox = [];

  String get rootBlockId => _rootBlockId;
  bool get isReady => _ready;

  /// Resolves once the session has bootstrapped (cold start complete).
  Future<void> get ready => _readyCompleter.future;

  /// Resolves once every pushed diff has been acked by the server (or [timeout]
  /// elapses). The §6 migrator awaits this before disposing, so the server folds
  /// all replayed migration ops before the socket closes (otherwise a `dispose`
  /// mid-flight would silently drop the document's tail content).
  Future<void> drainOutbox({
    Duration timeout = const Duration(seconds: 15),
  }) async {
    final deadline = DateTime.now().add(timeout);
    while (!_disposed && DateTime.now().isBefore(deadline)) {
      if (_channel != null && _ready) _flushOutbox();
      if (_outbox.isEmpty && _ackCount >= _pushCount) return;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
  }

  /// The full document as a flat block list (tree order) — for callers that
  /// rebuild a snapshot/bootstrap from the live replica.
  List<Map<String, dynamic>> allBlocks() {
    final doc = _doc;
    if (doc == null) return const [];
    return (jsonDecode(doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();
  }

  /// Current editor nodes (root block's children, in order).
  List<Map<String, dynamic>> childBlocks() {
    final doc = _doc;
    if (doc == null) return const [];
    final all =
        (jsonDecode(doc.toBlocksJson()) as List).cast<Map<String, dynamic>>();
    final byId = {for (final b in all) b['id'] as String: b};
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
      // Cold start: fetch the yrs base (the server also auto-sends a
      // `document.bootstrap` op snapshot first, which we ignore).
      _send({'type': 'sync.bootstrap'});
    } else {
      // Reconnect: keep our replica (it may hold unpushed edits), just catch up
      // from our cursor and flush the outbox.
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
          // Cold bootstrap.
          final doc = MicaDocument.fromStateWithClientId(
            bytes: base64.decode(b64),
            clientId: clientId,
          );
          if (doc == null) return;
          _doc = doc;
          _rootBlockId = doc.rootBlockId();
          _cursor = baseRid;
          _mirror.seedFrom(doc);
          _ready = true;
          if (!_readyCompleter.isCompleted) _readyCompleter.complete();
          onReady(_rootBlockId, childBlocks());
        } else if (baseRid > _cursor) {
          // Re-bootstrap: the stream was pruned past our cursor. Merge the base
          // (CRDT — our unpushed local edits survive) and fast-forward.
          existing.applyUpdate(update: base64.decode(b64));
          _cursor = baseRid;
          if (!_disposed) onRemoteBlocks(childBlocks());
        }
        // Catch up anything after the base, then push queued local edits.
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
        _ackCount++;
        final rid = (m['rid'] as num?)?.toInt();
        if (rid != null && rid > _cursor) _cursor = rid;
    }
  }

  bool _applyRemote(Map<String, dynamic> u) {
    final doc = _doc;
    if (doc == null) return false;
    final b64 = u['update'];
    if (b64 is! String) return false;
    final ok = doc.applyUpdate(update: base64.decode(b64));
    final rid = (u['rid'] as num?)?.toInt();
    if (rid != null && rid > _cursor) _cursor = rid;
    return ok;
  }

  /// Apply the editor's op batch to the local replica and push the resulting yrs
  /// diff to the cloud. The same op stream the offline backend consumes, so local
  /// and cloud editing behave identically.
  void applyLocalOps(List<DocOp> ops) {
    final doc = _doc;
    if (doc == null) return; // not bootstrapped yet
    final sv = doc.stateVector();
    for (final op in ops) {
      _mirror.apply(doc, op);
    }
    final diff = doc.encodeDiffSince(stateVector: sv);
    if (diff.isEmpty) return;
    _pushOrQueue(diff);
  }

  void _pushOrQueue(Uint8List diff) {
    if (_channel != null && _ready) {
      _sendPush(diff);
    } else {
      _outbox.add(diff);
    }
  }

  void _flushOutbox() {
    if (_channel == null || !_ready) return;
    final pending = List<Uint8List>.from(_outbox);
    _outbox.clear();
    for (final diff in pending) {
      _sendPush(diff);
    }
  }

  void _sendPush(Uint8List diff) {
    _pushCount++;
    _send({
      'type': 'sync.push',
      'payload': {'update': base64.encode(diff)},
    });
  }

  void _send(Map<String, dynamic> message) {
    _channel?.sink.add(jsonEncode(message));
  }

  void _onDone() {
    _channel = null;
    // Edits keep flowing into [_doc] + [_outbox]; a future connect() resumes.
  }

  void dispose() {
    _disposed = true;
    _sub?.cancel();
    _channel?.sink.close();
    _channel = null;
  }
}
