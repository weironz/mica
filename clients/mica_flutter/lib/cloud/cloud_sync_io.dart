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
import 'cloud_doc_store.dart';

export 'cloud_doc_store.dart';

/// A local yrs diff awaiting the server's ack. Tagged with a client id the
/// server echoes in `sync.ack` so an ack matches its specific diff even across
/// reconnect resends; `sent` avoids re-transmitting while a push is in flight.
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
    this.restoreUnacked,
    this.onPersistUnacked,
    this.persistence,
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

  /// Fired on an integrity fault the replica must not silently absorb — a remote
  /// update that won't apply, or a corrupt base (red line #1: never diverge
  /// silently). `reason` is a short code; `count` is the running fault total. The
  /// session self-heals by re-bootstrapping up to [_maxAutoReheal] times, then
  /// stops (circuit-break) and leaves it to the UI to prompt a reload.
  final void Function(String reason, int count)? onFault;

  /// Unacked diffs (raw yrs bytes) restored from local persistence at startup,
  /// so a crash / hard close doesn't lose edits the server never acked (C1).
  /// Replayed after (re)connect; the server folds duplicates idempotently.
  final List<Uint8List>? restoreUnacked;

  /// Persists the current unacked queue whenever it changes (debounced), so it
  /// survives a restart. Desktop wires this to the local store / prefs.
  final void Function(List<Uint8List> unacked)? onPersistUnacked;

  /// Local-first mirror for this cloud doc (P2 Phase 1, desktop): seeds the
  /// replica from the on-device store for offline read, and write-throughs the
  /// doc + sync cursor so it survives a restart. Null on web / when not mirrored.
  final CloudDocStore? persistence;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _sub;
  MicaDocument? _doc;
  final DocOpMirror _mirror = DocOpMirror();
  String _rootBlockId = '';

  /// Highest stream rid this replica has applied (per-document cursor).
  int _cursor = 0;
  bool _ready = false;
  bool _disposed = false;

  /// Integrity-fault accounting (red line #1). After [_maxAutoReheal] failed
  /// applies we stop auto-re-bootstrapping and rely on [onFault] → UI, rather
  /// than looping forever on a persistently bad base.
  int _faultCount = 0;
  static const int _maxAutoReheal = 3;

  /// Auto-reconnect (capped exponential backoff): a dropped socket / transient
  /// network loss re-syncs on its own instead of staying dead until the doc is
  /// reopened. No connectivity package — the backoff just retries until the
  /// network returns (in-house-first). Reset once a frame proves the link live.
  Timer? _reconnectTimer;
  int _reconnectAttempts = 0;

  /// Completes when the session first becomes ready (cold bootstrap done). Lets
  /// headless callers (e.g. the §6 migrator) await a usable replica.
  final Completer<void> _readyCompleter = Completer<void>();

  /// Diffs applied locally but not yet acked by the server. Sent when ready,
  /// re-sent on reconnect, removed when their id is acked — so [drainOutbox]
  /// knows exactly what's still in flight, and the queue is the unit of crash
  /// recovery (C1) and graceful drain (C2). Replaces the old fire-and-count
  /// outbox (which couldn't tell a specific diff apart from its ack).
  final List<_Pending> _unacked = [];
  int _pushSeq = 0;
  bool _restored = false;
  Timer? _persistTimer;

  /// Local-first mirror state (Phase 1): [_seeded] gates the one-time seed of the
  /// replica from the on-device store; [_saveTimer] debounces write-through.
  bool _seeded = false;
  Timer? _saveTimer;

  String get rootBlockId => _rootBlockId;
  bool get isReady => _ready;

  /// Resolves once the session has bootstrapped (cold start complete).
  Future<void> get ready => _readyCompleter.future;

  /// Resolves once every pushed diff has been acked by the server (or [timeout]
  /// elapses). The §6 migrator awaits this before disposing, so the server folds
  /// all replayed migration ops before the socket closes (otherwise a `dispose`
  /// mid-flight would silently drop the document's tail content).
  /// Returns `true` if every pushed diff was acked (fully drained), `false` if
  /// [timeout] elapsed with edits still in flight — so callers gating a
  /// close/switch on the drain can tell success from a timed-out drop (B4).
  Future<bool> drainOutbox({
    Duration timeout = const Duration(seconds: 15),
  }) async {
    final deadline = DateTime.now().add(timeout);
    while (!_disposed && DateTime.now().isBefore(deadline)) {
      if (_channel != null && _ready) _flushUnacked();
      if (_unacked.isEmpty) return true;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    return _unacked.isEmpty;
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
    _reconnectTimer?.cancel();
    _reconnectTimer = null;
    _sub?.cancel();
    _restoreUnackedOnce();
    // Local-first: render the persisted replica immediately (offline read),
    // BEFORE the socket — so a cold start with no network still shows the doc.
    _seedFromLocalOnce();
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
      // `document.bootstrap` op snapshot first, which we ignore). Any recovered
      // unacked diffs are replayed once the base arrives (see `sync.base`).
      _send({'type': 'sync.bootstrap'});
    } else {
      // Reconnect: keep our replica (it may hold unpushed edits), just catch up
      // from our cursor and resend everything still unacked.
      _send({
        'type': 'sync.pull',
        'payload': {'since_rid': _cursor},
      });
      _flushUnacked(resendAll: true);
    }
  }

  /// Seed the unacked queue from persisted state exactly once (crash recovery).
  void _restoreUnackedOnce() {
    if (_restored) return;
    _restored = true;
    final restore = restoreUnacked;
    if (restore == null || restore.isEmpty) return;
    for (final bytes in restore) {
      _unacked.add(_Pending('${_pushSeq++}', bytes));
    }
  }

  /// Seed the replica from the on-device store exactly once (Phase 1 offline
  /// read). If a persisted copy exists, decode it, fire [onReady] with its
  /// content immediately, and resume the stream from the saved cursor — so the
  /// doc opens with zero connectivity. A corrupt/absent copy falls through to the
  /// normal server bootstrap (`_doc` stays null → `connect` sends sync.bootstrap).
  void _seedFromLocalOnce() {
    if (_seeded || _doc != null || persistence == null) return;
    _seeded = true;
    final loaded = persistence!.load();
    if (loaded == null) return;
    final doc = MicaDocument.fromStateWithClientId(
      bytes: loaded.state,
      clientId: clientId,
    );
    if (doc == null) return; // corrupt local copy → cold-bootstrap from server
    _doc = doc;
    _rootBlockId = doc.rootBlockId();
    _cursor = loaded.cursor;
    _mirror.seedFrom(doc);
    _ready = true;
    if (!_readyCompleter.isCompleted) _readyCompleter.complete();
    onReady(_rootBlockId, childBlocks());
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
          // Cold bootstrap.
          final doc = MicaDocument.fromStateWithClientId(
            bytes: base64.decode(b64),
            clientId: clientId,
          );
          if (doc == null) {
            // A base we can't decode is an integrity fault, not a no-op — surface
            // it (and retry, capped) instead of leaving the session stuck empty.
            _onIntegrityFault('bad_base');
            return;
          }
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
          final ok = existing.applyUpdate(update: base64.decode(b64));
          if (!ok) {
            _onIntegrityFault('bad_base');
            return;
          }
          _cursor = baseRid;
          if (!_disposed) onRemoteBlocks(childBlocks());
        }
        // A good base means recovery worked — reset the consecutive-fault count
        // so the circuit-breaker (B3) measures a genuine stuck streak, not
        // transient faults spread across a healthy session.
        _faultCount = 0;
        // Catch up anything after the base, then push queued local edits
        // (including any recovered from a prior crash).
        _send({
          'type': 'sync.pull',
          'payload': {'since_rid': _cursor},
        });
        _flushUnacked(resendAll: true);
        _saveLocalSoon(); // persist the fresh/merged base + cursor
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
        // B2 (verified catch-up): the server caps each pull, so a non-empty batch
        // may be truncated. If we made forward progress, keep pulling until the
        // stream after our cursor is empty — no silently-dropped tail. Gated on
        // cursor advancing so a batch that all failed to apply (B1 re-bootstraps)
        // can't loop.
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
        if (rid != null && rid > _cursor) {
          _cursor = rid;
          _saveLocalSoon(); // persist the cursor our own acked edit advanced
        }
    }
  }

  bool _applyRemote(Map<String, dynamic> u) {
    final doc = _doc;
    if (doc == null) return false;
    final b64 = u['update'];
    if (b64 is! String) return false;
    final ok = doc.applyUpdate(update: base64.decode(b64));
    if (!ok) {
      // Red line #1: a remote update we can't apply is an integrity fault, not
      // something to skip. Do NOT advance the cursor past it — that would
      // silently drop the content it carried and leave a hole in the stream.
      // Self-heal by re-bootstrapping from the server's folded base (which
      // already incorporates this update), which CRDT-merges so unpushed local
      // edits survive. Capped so a persistently bad base can't loop.
      _onIntegrityFault('bad_remote_update');
      return false;
    }
    final rid = (u['rid'] as num?)?.toInt();
    if (rid != null && rid > _cursor) _cursor = rid;
    _saveLocalSoon(); // persist the merged remote update + advanced cursor
    return true;
  }

  /// Handle an integrity fault (red line #1): count it, notify [onFault], and —
  /// up to [_maxAutoReheal] times — request a fresh folded base to self-heal.
  /// Past the cap we stop retrying (circuit-break) and rely on the UI to prompt
  /// a reload, rather than diverging silently or looping on a bad base.
  void _onIntegrityFault(String reason) {
    _faultCount++;
    onFault?.call(reason, _faultCount);
    if (_channel != null && !_disposed && _faultCount <= _maxAutoReheal) {
      _send({'type': 'sync.bootstrap'});
    }
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
    _enqueue(diff);
  }

  /// Queue a diff as unacked, persist the queue, and send it if we're connected.
  void _enqueue(Uint8List diff) {
    final p = _Pending('${_pushSeq++}', diff);
    _unacked.add(p);
    _persistSoon();
    _saveLocalSoon(); // persist the local edit for offline durability
    if (_channel != null && _ready) _sendPush(p);
  }

  /// Push unacked diffs. [resendAll] re-sends everything (after a (re)connect,
  /// where in-flight pushes may have been lost); otherwise only the not-yet-sent
  /// ones go out, so a live drain doesn't spam duplicates.
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

  /// Debounced persistence of the unacked queue (crash recovery, C1).
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

  /// Debounced write-through of the replica + cursor to the on-device store
  /// (Phase 1). Coalesces edit/remote-update bursts so we re-encode the doc once
  /// per idle window, not per keystroke — offline durability without I/O churn.
  void _saveLocalSoon() {
    if (persistence == null || _doc == null) return;
    _saveTimer?.cancel();
    _saveTimer = Timer(const Duration(milliseconds: 400), _saveLocalNow);
  }

  void _saveLocalNow() {
    _saveTimer?.cancel();
    _saveTimer = null;
    final doc = _doc;
    final store = persistence;
    if (doc == null || store == null) return;
    store.save(doc.encodeState(), _cursor);
  }

  void _send(Map<String, dynamic> message) {
    _channel?.sink.add(jsonEncode(message));
  }

  void _onDone() {
    _channel = null;
    // Socket dropped (server close / network loss). Edits keep flowing into
    // [_doc] + [_unacked]; auto-reconnect resumes and resends what's unacked.
    _scheduleReconnect();
  }

  /// Schedule a reconnect with capped exponential backoff (0.5s → 30s). No-op if
  /// disposed, already connected, or a retry is already pending.
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

  /// Graceful teardown (C2): flush + await acks (up to [timeout]) before closing,
  /// so a doc switch / workspace change / sign-out doesn't hard-drop unacked
  /// edits. Returns whether the outbox fully drained. Fire-and-forget from
  /// synchronous teardown paths; the un-awaitable app-close case still needs the
  /// local-snapshot fallback (C1) for a hard guarantee.
  Future<bool> drainAndDispose({
    Duration timeout = const Duration(seconds: 4),
  }) async {
    final drained = await drainOutbox(timeout: timeout);
    dispose();
    return drained;
  }

  void dispose() {
    // Best-effort: if the socket is still live, transmit anything not yet sent
    // before we close. Does not wait for acks — [drainAndDispose] is the
    // awaitable path. Then flush the unacked queue to persistence synchronously
    // so a hard close still leaves a recoverable record (C1).
    if (!_disposed && _channel != null && _ready) {
      _flushUnacked();
    }
    _persistNow();
    _saveLocalNow(); // hard-close guarantee: flush the replica to disk (C1)
    _disposed = true;
    _reconnectTimer?.cancel();
    _sub?.cancel();
    _channel?.sink.close();
    _channel = null;
  }
}
