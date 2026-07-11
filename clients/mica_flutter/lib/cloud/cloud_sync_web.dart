// P2-M4 W3: the WEB cloud editing session — same WS sync protocol as the desktop
// [CloudSyncSession] (cloud_sync_io.dart), but the CRDT replica is a JS `yjs`
// document ([MicaYDoc]) instead of the Rust `yrs` FFI. yjs and yrs are
// wire-compatible (verified W1/W2), so a web client and a desktop client
// editing the same document converge with no translation.
//
// P4-2: web is local-first too — when [persistence] is set (an IndexedDB-backed
// [WebIdbDocStore] from the platform factory), this session runs the SAME
// append-log machinery as the desktop: seed-from-store offline read, durable
// outbox with contiguous-prefix ack tracking, remote-update log, timerless
// compaction. When IndexedDB is unavailable (private mode), persistence is null
// and the session falls back to the legacy in-memory queue + prefs, as before.
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
/// echoes in `sync.ack`. Parity with the desktop session (legacy path — with a
/// store the durable outbox replaces this).
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
  /// actor id; a fresh actor per page load is CRDT-correct (convergence holds),
  /// so the web store doesn't pin one.
  final BigInt clientId;

  final void Function(String rootBlockId, List<Map<String, dynamic>> blocks)
  onReady;
  final void Function(List<Map<String, dynamic>> blocks) onRemoteBlocks;

  /// Integrity-fault hook — parity with the desktop session (red line #1).
  final void Function(String reason, int count)? onFault;

  /// Fired once per session the first time a valid frame arrives — "we are
  /// online". Web has no offline-nav fallback to leave (that recovery is
  /// desktop-only), but firing it keeps the contract identical across platforms.
  final void Function()? onServerConnected;

  /// Crash-recovery parity (C1), legacy path only: unacked diffs restored at
  /// startup + a persist callback fired when the queue changes. Unused when
  /// [persistence] is set (the durable outbox IS the crash record).
  final List<Uint8List>? restoreUnacked;
  final void Function(List<Uint8List> unacked)? onPersistUnacked;

  /// Local-first mirror for this cloud doc (P4-2, IndexedDB): seeds the replica
  /// from the browser store for offline read and holds the durable outbox +
  /// remote log. Null when IndexedDB is unavailable → online-only, as before.
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

  /// Append-log path (`persistence != null`) — mirrors cloud_sync_io.dart:
  /// highest outbox clock pushed on THIS connection (no per-entry sent flag).
  int _sentThroughClock = 0;

  /// Outbox clocks acked out of contiguous order; `pushed_clock` advances only
  /// through the contiguous acked prefix (an error-framed lower clock must
  /// never be skipped — that would drop it from `outboxAfter` = silent loss).
  final Set<int> _ackedAhead = {};

  /// Consecutive push rejections without contiguous progress; past the budget
  /// we stall active pushing (poison-edit circuit breaker, cleared on
  /// reconnect). Kept > 3 so the UI's fault-banner threshold still fires.
  int _pushRejects = 0;
  static const int _maxPushRejects = 5;
  bool _pushStalled = false;
  bool _restored = false;
  Timer? _persistTimer;

  /// Local-first mirror state: gates the one-time seed from the browser store.
  bool _seeded = false;
  bool _sawServerFrame = false;

  /// Auto-reconnect with capped backoff — parity with the desktop session.
  Timer? _reconnectTimer;
  int _reconnectAttempts = 0;

  String get rootBlockId => _rootBlockId;
  bool get isReady => _ready;

  /// Durable-outbox mode: the browser store's append-log is the outbox
  /// (survives reload/crash), replacing the in-memory queue + prefs.
  bool get _useAppendLog => persistence != null;

  bool get _outboxEmpty => _useAppendLog
      ? persistence!.outboxAfter(persistence!.cursor().pushedClock).isEmpty
      : _unacked.isEmpty;

  // §6 migration parity (desktop-only feature; present here so main.dart compiles
  // for web — the web build never drives a migration since local offline is
  // native-only).
  final Completer<void> _readyCompleter = Completer<void>();
  Future<void> get ready => _readyCompleter.future;

  /// Best-effort flush + report whether the outbox is empty (B4 parity).
  Future<bool> drainOutbox({Duration timeout = const Duration(seconds: 15)}) async {
    final deadline = DateTime.now().add(timeout);
    while (!_disposed && DateTime.now().isBefore(deadline)) {
      if (_channel != null && _ready) _flushUnacked();
      if (_outboxEmpty) return true;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    return _outboxEmpty;
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
    // New connection: per-connection push bookkeeping resets so a resendAll
    // below pushes the whole un-acked outbox afresh (mirrors io).
    _sentThroughClock = 0;
    _ackedAhead.clear();
    _pushRejects = 0;
    _pushStalled = false;
    _restoreUnackedOnce();
    // Local-first: render the persisted replica immediately (offline read),
    // BEFORE the socket — a cold reload with no network still shows the doc.
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
      _send({'type': 'sync.bootstrap'});
    } else {
      _send({'type': 'sync.pull', 'payload': _pullPayload()});
      _flushUnacked(resendAll: true);
    }
  }

  /// The sync.pull payload — P4-3 parity with io: advertise the replica's
  /// state vector so a prune-forced re-bootstrap answers with the minimal diff
  /// instead of the full doc. The integrity-fault heal deliberately requests a
  /// FULL base (no sv).
  Map<String, dynamic> _pullPayload() {
    final doc = _doc;
    return {
      'since_rid': _cursor,
      if (doc != null) 'sv': base64.encode(doc.stateVector()),
    };
  }

  /// No-op in append-log mode — the browser store IS the durable outbox.
  void _restoreUnackedOnce() {
    if (_restored) return;
    _restored = true;
    if (_useAppendLog) return;
    final restore = restoreUnacked;
    if (restore == null || restore.isEmpty) return;
    for (final bytes in restore) {
      _unacked.add(_Pending('${_pushSeq++}', bytes));
    }
  }

  /// Seed the replica from the browser store exactly once (offline read). A
  /// corrupt/absent copy falls through to the normal server bootstrap
  /// ([MicaYDoc.fromState] throws on bytes it can't apply → catch → cold path).
  void _seedFromLocalOnce() {
    if (_seeded || _doc != null || persistence == null) return;
    _seeded = true;
    final loaded = persistence!.load();
    if (loaded == null) return;
    MicaYDoc doc;
    try {
      doc = MicaYDoc.fromState(loaded.state);
    } catch (_) {
      return; // corrupt local copy → cold-bootstrap from server
    }
    _doc = doc;
    _rootBlockId = doc.rootBlockId();
    _cursor = loaded.cursor;
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
    if (!_sawServerFrame) {
      _sawServerFrame = true;
      onServerConnected?.call();
    }
    switch (m['type']) {
      case 'sync.base':
        final b64 = m['base'];
        if (b64 is! String) return;
        final baseRid = (m['base_rid'] as num?)?.toInt() ?? 0;
        final existing = _doc;
        if (existing == null) {
          final MicaYDoc doc;
          try {
            doc = MicaYDoc.fromState(base64.decode(b64));
          } catch (e, st) {
            _onIntegrityFault('bad_base');
            return;
          }
          _doc = doc;
          _rootBlockId = doc.rootBlockId();
          _cursor = baseRid;
          _ready = true;
          if (!_readyCompleter.isCompleted) _readyCompleter.complete();
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
        _send({'type': 'sync.pull', 'payload': _pullPayload()});
        _flushUnacked(resendAll: true);
        _saveBaseNow(); // a base IS a snapshot — persist it now (rare event)
      case 'sync.updates':
        final ups = m['updates'];
        final before = _cursor;
        var changed = false;
        final applied = <({int rid, Uint8List update})>[];
        if (ups is List) {
          for (final u in ups) {
            if (u is! Map) continue;
            final item = u.cast<String, dynamic>();
            if (_applyRemote(item, persist: false)) {
              changed = true;
              final rid = (item['rid'] as num?)?.toInt();
              final b64 = item['update'];
              if (rid != null && b64 is String) {
                applied.add((rid: rid, update: base64.decode(b64)));
              }
            }
          }
        }
        // One store write for the whole batch (mirrors the desktop's
        // one-transaction catch-up persist).
        if (applied.isNotEmpty) {
          final store = persistence;
          if (store != null && !store.appendRemoteBatch(applied)) {
            _healPersistFailure();
          }
          _maybeCompact();
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
          _send({'type': 'sync.pull', 'payload': _pullPayload()});
        }
      case 'sync.update':
        if (_applyRemote(m) && !_disposed) onRemoteBlocks(childBlocks());
      case 'sync.ack':
        final ackId = m['ack_id'];
        final rid = (m['rid'] as num?)?.toInt();
        if (_useAppendLog) {
          // The ack id is the pushed diff's monotonic clock. Advance
          // pushed_clock ONLY through the contiguous acked prefix — a push can
          // be answered with `error` instead, so a higher ack does NOT prove
          // every lower clock was folded (mirrors io).
          final clock = ackId is String ? int.tryParse(ackId) : null;
          if (clock != null) {
            final pushedBefore = persistence!.cursor().pushedClock;
            if (clock > pushedBefore) {
              _ackedAhead.add(clock);
              var pushed = pushedBefore;
              while (_ackedAhead.remove(pushed + 1)) {
                pushed++;
              }
              if (pushed != pushedBefore) {
                persistence!.advance(pushedClock: pushed);
                // Reset the retry budget only on real contiguous PROGRESS.
                _pushRejects = 0;
              }
            }
          }
        } else if (ackId is String) {
          final before = _unacked.length;
          _unacked.removeWhere((p) => p.id == ackId);
          if (_unacked.length != before) _persistSoon();
        }
        if (rid != null && rid > _cursor) {
          _cursor = rid;
          final store = persistence;
          if (store != null && rid > store.cursor().lastSyncedRid) {
            store.advance(lastSyncedRid: rid);
          }
        }
      case 'error':
        // A push was rejected (not acked). The rejected clock stays in the
        // outbox (contiguous pushed_clock never passed it); re-enable and retry
        // it, bounded so a permanent rejection can't spin (mirrors io). The
        // legacy path keeps its per-id retry-on-reconnect; nothing to do there.
        final errId = m['ack_id'];
        if (_useAppendLog && errId is String) {
          final clock = int.tryParse(errId);
          if (clock != null) {
            if (clock - 1 < _sentThroughClock) _sentThroughClock = clock - 1;
            if (_pushRejects < _maxPushRejects) {
              _pushRejects++;
              _flushUnacked();
            } else {
              _pushStalled = true;
              onFault?.call('push_rejected', _pushRejects);
            }
          }
        }
    }
  }

  bool _applyRemote(Map<String, dynamic> u, {bool persist = true}) {
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
    // P4-2: durably append the remote update the moment it applies (its own
    // log + lastSyncedRid together). A rid-less update is deliberately NOT
    // persisted: its cursor didn't advance, so the next pull re-delivers it.
    if (rid != null && persist) {
      _persistRemote(rid, base64.decode(b64));
      _maybeCompact();
    }
    return true;
  }

  /// Persist one remote update; on a FAILED write, self-heal by snapshotting
  /// the live in-memory doc as the base (it is authoritative and contains the
  /// update) — otherwise a later compact would rebuild from the store and fold
  /// the hole into the base permanently. Mirrors io.
  void _persistRemote(int rid, Uint8List bytes) {
    final store = persistence;
    if (store == null) return;
    if (!store.appendRemote(rid, bytes)) _healPersistFailure();
  }

  int _persistFails = 0;

  void _healPersistFailure() {
    _saveBaseNow(); // live doc → base: covers whatever the log write missed
    onFault?.call('persist_failed', ++_persistFails);
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
    if (_useAppendLog) {
      // Durable append (survives reload/crash); its monotonic `clock` is the
      // push id the server echoes in `sync.ack`. Mirrors io.
      final clock = persistence!.appendOutbox(diff);
      _maybeCompact();
      if (_channel != null && _ready && !_pushStalled) {
        _sendPushRaw(clock.toString(), diff);
        _sentThroughClock = clock;
      }
      return;
    }
    final p = _Pending('${_pushSeq++}', diff);
    _unacked.add(p);
    _persistSoon();
    if (_channel != null && _ready) _sendPush(p);
  }

  void _flushUnacked({bool resendAll = false}) {
    if (_channel == null || !_ready) return;
    if (_useAppendLog) {
      // While stalled on a poison edit, only a (re)connect's resendAll retries.
      if (_pushStalled && !resendAll) return;
      final pushed = persistence!.cursor().pushedClock;
      final floor =
          resendAll || pushed > _sentThroughClock ? pushed : _sentThroughClock;
      for (final e in persistence!.outboxAfter(floor)) {
        _sendPushRaw(e.clock.toString(), e.bytes);
        if (e.clock > _sentThroughClock) _sentThroughClock = e.clock;
      }
      return;
    }
    for (final p in _unacked) {
      if (resendAll || !p.sent) _sendPush(p);
    }
  }

  void _sendPush(_Pending p) {
    p.sent = true;
    _sendPushRaw(p.id, p.bytes);
  }

  void _sendPushRaw(String id, Uint8List bytes) {
    _send({
      'type': 'sync.push',
      'id': id,
      'payload': {'update': base64.encode(bytes)},
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

  /// Persist the full replica as the base snapshot NOW — runs only when a
  /// server base arrives (once per bootstrap/re-bootstrap) or as self-heal;
  /// steady-state persistence is pure append-log. Mirrors io.
  void _saveBaseNow() {
    final doc = _doc;
    final store = persistence;
    if (doc == null || store == null) return;
    store.save(doc.encodeState(), _cursor);
  }

  int _appendsSinceCompactCheck = 0;
  static const int _compactCheckEvery = 32;
  static const int _compactThreshold = 256;

  /// Bound the logs without timers: every [_compactCheckEvery] appends check
  /// the combined log size; past [_compactThreshold], fold base + logs into a
  /// fresh base (un-pushed outbox tail survives). Mirrors io.
  void _maybeCompact() {
    final store = persistence;
    if (store == null) return;
    if (++_appendsSinceCompactCheck < _compactCheckEvery) return;
    _appendsSinceCompactCheck = 0;
    final sizes = store.logSizes();
    if (sizes.local + sizes.remote > _compactThreshold) _compactNow();
  }

  void _compactNow() {
    final store = persistence;
    if (store == null || _doc == null) return;
    store.compact();
  }

  void _send(Map<String, dynamic> message) {
    try {
      _channel?.sink.add(jsonEncode(message));
    } catch (_) {
      // Mid-drop / refused socket: dropping the frame is safe — the durable
      // outbox re-pushes on the next (re)connect. Never crash the session.
    }
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
    _compactNow(); // hard-close: fold base+logs so the store reopens fastest
    _disposed = true;
    _reconnectTimer?.cancel();
    _sub?.cancel();
    _channel?.sink.close();
    _channel = null;
  }
}
