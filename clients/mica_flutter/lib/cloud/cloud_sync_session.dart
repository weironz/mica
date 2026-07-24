// P2-M4.5 / 丁-1: THE live cloud editing session for one document — one shared
// protocol state machine for every platform.
//
// It owns a [SyncDocReplica] (the device's CRDT replica — Rust yrs over FFI on
// desktop, JS yjs on web; wire-compatible, chosen by the conditional import
// below), speaks the WS sync protocol added in M4.4 (sync.bootstrap/pull/push +
// sync.update), pushes local editor ops as CRDT diffs, and merges remote
// updates — firing [onRemoteBlocks] so the editor can reconcile. CRDT merge
// makes concurrent edits converge with no central locking.
//
// Local-first (P2 Phase 1 desktop / P4-2 web): when [persistence] is set (the
// platform's [CloudDocStore]), the session runs the append-log machinery —
// seed-from-store offline read, durable outbox with contiguous-prefix ack
// tracking, remote-update log, timerless compaction. Without a store it falls
// back to the legacy in-memory queue + prefs.
//
// History: until 2026-07-21 this file existed twice (cloud_sync_io.dart /
// cloud_sync_web.dart) with 407 non-comment lines byte-identical, red line #1
// kept in sync by hand. The engine seam ([SyncDocReplica]) ended that; keep
// engine types out of this file.
import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';

import 'package:flutter/foundation.dart' show visibleForTesting;
import 'package:web_socket_channel/web_socket_channel.dart';

import 'cloud_doc_store.dart';
import 'sync_doc_replica.dart';
import 'sync_status.dart';
import 'sync_replica_io.dart' if (dart.library.html) 'sync_replica_web.dart';

export 'cloud_doc_store.dart';
export 'sync_doc_replica.dart' show DocOp, SyncDocReplica;
export 'sync_status.dart';

/// A local CRDT diff awaiting the server's ack. Tagged with a client id the
/// server echoes in `sync.ack` so an ack matches its specific diff even across
/// reconnect resends; `sent` avoids re-transmitting while a push is in flight.
/// Legacy path — with a store the durable outbox replaces this.
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
    this.onSyncPhase,
    this.restoreUnacked,
    this.onPersistUnacked,
    this.persistence,
  });

  /// The document WebSocket URI (already carrying the auth token).
  final Uri uri;

  /// This device's stable CRDT actor id. Desktop pins the replica to it (from
  /// the local store identity) so all of a device's edits share one actor
  /// across sessions; the web adapter ignores it — yjs assigns a per-session
  /// actor, which is CRDT-correct (convergence holds).
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

  /// Fired once per session the first time a valid frame arrives from the server
  /// — a definitive "we are online" signal. Used to leave the P1c offline-nav
  /// fallback (refetch the authoritative workspace list once reachable again);
  /// P4-2 gave web one too, wired to `_recoverOnlineNav`.
  final void Function()? onServerConnected;

  /// Fired when the derived sync phase changes (synced / syncing / offline) —
  /// drives the quiet status icon. Fires only on a real transition, not every
  /// frame. Null → the UI opts out. Pure derivation lives in [deriveSyncPhase].
  final void Function(SyncPhase phase)? onSyncPhase;

  /// Unacked diffs (raw update bytes) restored from local persistence at
  /// startup, so a crash / hard close doesn't lose edits the server never acked
  /// (C1). Replayed after (re)connect; the server folds duplicates idempotently.
  /// Legacy path only — unused when [persistence] is set (the durable outbox IS
  /// the crash record).
  final List<Uint8List>? restoreUnacked;

  /// Persists the current unacked queue whenever it changes (debounced), so it
  /// survives a restart. Legacy path only (prefs-backed).
  final void Function(List<Uint8List> unacked)? onPersistUnacked;

  /// Local-first mirror for this cloud doc: seeds the replica from the
  /// on-device store for offline read, and holds the durable outbox + remote
  /// log + sync cursor so they survive a restart. Desktop wires the SQLite
  /// store, web the IndexedDB store; null → online-only (e.g. private mode).
  final CloudDocStore? persistence;

  WebSocketChannel? _channel;
  StreamSubscription<dynamic>? _sub;
  SyncDocReplica? _doc;
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
  /// Append-log path (`persistence != null`): the highest outbox `clock` pushed
  /// on the current WS connection, so a live re-flush skips still-in-flight diffs
  /// (the append-log has no per-entry `sent` flag). Reset on each (re)connect.
  int _sentThroughClock = 0;
  /// Append-log path: outbox clocks acked out of contiguous order (a lower clock
  /// errored / isn't acked yet). `pushed_clock` advances only through the
  /// contiguous acked prefix, so an un-acked lower clock is never skipped
  /// (skipping it drops it from `outboxAfter` = silent server-side loss).
  final Set<int> _ackedAhead = {};
  /// Consecutive push rejections without contiguous progress — bounds the
  /// re-push retry so a permanent rejection (e.g. permission) can't spin.
  /// NOTE: kept > 3 so the UI's fault-banner threshold (main.dart, count > 3)
  /// still surfaces `push_rejected`; lowering it silently hides the banner.
  int _pushRejects = 0;
  static const int _maxPushRejects = 5;
  /// Set once the retry budget is exhausted (a push is permanently rejected):
  /// stop actively pushing so we don't grow `_ackedAhead` / re-flood the server
  /// with a poison edit that can never ack. Edits still append durably to the
  /// outbox and are retried on the next (re)connect, which clears this.
  bool _pushStalled = false;
  bool _restored = false;
  Timer? _persistTimer;

  /// Local-first mirror state: [_seeded] gates the one-time seed of the replica
  /// from the on-device store.
  bool _seeded = false;
  bool _sawServerFrame = false;

  String get rootBlockId => _rootBlockId;
  bool get isReady => _ready;

  /// Durable-outbox mode: the platform store's append-log is the outbox
  /// (survives restart/crash), replacing the in-memory queue + prefs used
  /// when no store is available.
  bool get _useAppendLog => persistence != null;

  /// True when nothing local is still waiting for a server ack.
  bool get _outboxEmpty => _useAppendLog
      ? persistence!.outboxAfter(persistence!.cursor().pushedClock).isEmpty
      : _unacked.isEmpty;

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
      if (_outboxEmpty) return true;
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    return _outboxEmpty;
  }

  /// The full document as a flat block list (tree order) — for callers that
  /// rebuild a snapshot/bootstrap from the live replica.
  List<Map<String, dynamic>> allBlocks() => _doc?.toBlocks() ?? const [];

  /// Current editor nodes (root block's children, in order).
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
    // New connection: nothing has been (re)sent on it yet, so a resendAll below
    // pushes the whole un-acked outbox afresh. Out-of-order ack bookkeeping and
    // the rejection budget are per-connection.
    _sentThroughClock = 0;
    _ackedAhead.clear();
    _pushRejects = 0;
    _pushStalled = false;
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
      // Cold start: fetch the CRDT base (the server also auto-sends a
      // `document.bootstrap` op snapshot first, which we ignore). Any recovered
      // unacked diffs are replayed once the base arrives (see `sync.base`).
      _send({'type': 'sync.bootstrap'});
    } else {
      // Reconnect: keep our replica (it may hold unpushed edits), just catch up
      // from our cursor and resend everything still unacked.
      _send({'type': 'sync.pull', 'payload': _pullPayload()});
      _flushUnacked(resendAll: true);
    }
  }

  /// The sync.pull payload. P4-3: alongside the stream cursor we advertise the
  /// replica's state vector, so a prune-forced re-bootstrap answers with the
  /// minimal diff instead of the full doc (`sync.base` carries either — both
  /// are CRDT updates, applied identically). An old server ignores `sv`; the
  /// deliberate exception is the integrity-fault heal, which requests a FULL
  /// base (no sv) since a corrupt replica's sv is the wrong basis for a diff.
  Map<String, dynamic> _pullPayload() {
    final doc = _doc;
    return {
      'since_rid': _cursor,
      if (doc != null) 'sv': base64.encode(doc.stateVector()),
    };
  }

  /// Seed the unacked queue from persisted state exactly once (crash recovery).
  /// No-op in append-log mode — the on-device log IS the durable outbox, so
  /// there's nothing to restore from the (unused) prefs queue.
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

  /// Seed the replica from the on-device store exactly once (offline read). If
  /// a persisted copy exists, decode it, fire [onReady] with its content
  /// immediately, and resume the stream from the saved cursor — so the doc
  /// opens with zero connectivity. A corrupt/absent copy falls through to the
  /// normal server bootstrap (`_doc` stays null → `connect` sends
  /// sync.bootstrap).
  void _seedFromLocalOnce() {
    if (_seeded || _doc != null || persistence == null) return;
    _seeded = true;
    final loaded = persistence!.load();
    if (loaded == null) return;
    final doc = replicaFromState(loaded.state, clientId);
    if (doc == null) return; // corrupt local copy → cold-bootstrap from server
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
    if (!_online) {
      _online = true;
      _updateSyncPhase(); // offline → online (synced or syncing per outbox)
    }
    // First contact with the server this session → surface "we are online" once
    // (lets the offline-nav fallback refetch the authoritative nav).
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
          // Cold bootstrap.
          final doc = replicaFromState(base64.decode(b64), clientId);
          if (doc == null) {
            // A base we can't decode is an integrity fault, not a no-op — surface
            // it (and retry, capped) instead of leaving the session stuck empty.
            _onIntegrityFault('bad_base');
            return;
          }
          _doc = doc;
          _rootBlockId = doc.rootBlockId();
          _cursor = baseRid;
          _ready = true;
          if (!_readyCompleter.isCompleted) _readyCompleter.complete();
          onReady(_rootBlockId, childBlocks());
        } else {
          // Re-bootstrap: the stream was pruned past our cursor. Merge the base
          // (CRDT — our unpushed local edits survive) UNCONDITIONALLY, then only
          // fast-forward the cursor when the base is genuinely ahead. The merge
          // must NOT be gated on `baseRid > _cursor`: a single `sync.update`
          // frame can advance `_cursor` past a rid whose content we couldn't
          // fully integrate (the engine stores an update with missing deps as
          // *pending* and returns Ok, so the cursor jumps but the bytes aren't
          // live). If a broadcast races ahead of the pull reply on reconnect,
          // the cursor can sit at/above base_rid while the pruned range
          // 11..base_rid is still missing; gating the merge would skip the very
          // base that backfills it — permanent silent divergence (red line #1),
          // persisted by P4-1 and surviving restart. Re-merging an equal/older
          // base is a CRDT no-op.
          final ok = existing.applyUpdate(base64.decode(b64));
          if (!ok) {
            _onIntegrityFault('bad_base');
            return;
          }
          if (baseRid > _cursor) _cursor = baseRid;
          if (!_disposed) onRemoteBlocks(childBlocks());
        }
        // A good base means recovery worked — reset the consecutive-fault count
        // so the circuit-breaker (B3) measures a genuine stuck streak, not
        // transient faults spread across a healthy session.
        _faultCount = 0;
        // Catch up anything after the base, then push queued local edits
        // (including any recovered from a prior crash).
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
        // One transaction for the whole batch — a 1000-row catch-up pull costs
        // one journal sync, not a thousand (P4-1 review: reconnect storm).
        if (applied.isNotEmpty) {
          final store = persistence;
          if (store != null && !store.appendRemoteBatch(applied)) {
            _healPersistFailure();
          }
          _maybeCompact();
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
          _send({'type': 'sync.pull', 'payload': _pullPayload()});
        }
      case 'sync.update':
        if (_applyRemote(m) && !_disposed) onRemoteBlocks(childBlocks());
      case 'sync.ack':
        final ackId = m['ack_id'];
        final rid = (m['rid'] as num?)?.toInt();
        if (_useAppendLog) {
          // The ack id is the pushed diff's monotonic clock. Advance pushed_clock
          // ONLY through the contiguous acked prefix — a push can be answered
          // with an `error` (below) instead of an ack, so a higher ack does NOT
          // prove every lower clock was folded. Skipping an un-acked lower clock
          // would drop it from `outboxAfter(pushed_clock)` = silent server loss.
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
                // Reset the retry budget only on real contiguous PROGRESS — not on
                // any ack — else a permanent rejection of a low clock would loop
                // forever while higher clocks keep acking.
                _pushRejects = 0;
                _updateSyncPhase(); // outbox drained a step → maybe now synced
              }
            }
          }
        } else if (ackId is String) {
          final before = _unacked.length;
          _unacked.removeWhere((p) => p.id == ackId);
          if (_unacked.length != before) {
            _persistSoon();
            _updateSyncPhase(); // outbox drained → maybe now synced
          }
        }
        if (rid != null && rid > _cursor) {
          _cursor = rid;
          // Persist the advanced stream position (one tiny cursor write; the
          // update bytes are ours — already in the base/outbox).
          final store = persistence;
          if (store != null && rid > store.cursor().lastSyncedRid) {
            store.advance(lastSyncedRid: rid);
          }
        }
      case 'error':
        // Two shapes share `type:'error'`. First, the server's lag notice: the
        // per-document broadcast channel overflowed and dropped updates for us
        // (`code:'client_out_of_date'`, no ack_id — it's a stream-state signal,
        // not a per-push rejection). This branch used to only recognise a push
        // rejection (String ack_id), so the lag notice fell through and was
        // dropped — leaving the replica silently behind (red line #1). Catch up
        // now with a fresh pull/bootstrap.
        final errId = m['ack_id'];
        if (errId == null && m['code'] == 'client_out_of_date') {
          _resyncFromLag();
          break;
        }
        // Otherwise: a push we sent was rejected (not acked) — e.g. transient
        // server-side contention, or a permanent permission error. The rejected
        // clock stays in the outbox (contiguous pushed_clock never passed it);
        // re-enable and retry it, bounded so a permanent rejection can't spin,
        // then surface via onFault. (The legacy path keeps its per-id
        // retry-on-reconnect; nothing to do here.)
        if (_useAppendLog && errId is String) {
          final clock = int.tryParse(errId);
          if (clock != null) {
            if (clock - 1 < _sentThroughClock) _sentThroughClock = clock - 1;
            if (_pushRejects < _maxPushRejects) {
              _pushRejects++;
              _flushUnacked();
            } else {
              // Give up actively retrying this poison edit: stall pushing (edits
              // still append durably; a reconnect retries) and surface it.
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
    // P4-1: durably append the remote update the MOMENT it applies (its own
    // log + lastSyncedRid in one transaction) — replaces the debounced full-
    // snapshot write-through, so there is no 400ms crash window anymore. A
    // rid-less update is deliberately NOT persisted: its cursor didn't
    // advance, so the next pull re-delivers it (idempotent); today every
    // server path carries a rid — if a future protocol change breaks that,
    // this comment is the tripwire.
    if (rid != null && persist) {
      _persistRemote(rid, base64.decode(b64));
      _maybeCompact();
    }
    return true;
  }

  /// Persist one remote update; on a FAILED write, self-heal by snapshotting
  /// the live in-memory doc as the base (it is authoritative and contains the
  /// update) — otherwise a later compact would rebuild from disk and fold the
  /// hole into the base permanently (P4-1 review CRITICAL). Surfaced via
  /// [onFault] so a persistently failing store is observable.
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

  DateTime? _lastLagResync;

  /// Test-only counter of lag-triggered resyncs; bumped even when offline (no
  /// channel to actually send on) so a test can assert the frame was recognised.
  @visibleForTesting
  int debugLagResyncCount = 0;

  /// Catch up after the server flags us out of date (broadcast lag). We still
  /// hold our replica, so a `sync.pull` carrying our cursor + state vector
  /// re-delivers just the missing diff; a cold replica (no doc) has no basis for
  /// a diff → full `sync.bootstrap`. Debounced so a burst of lag notices costs
  /// one catch-up, not a storm.
  void _resyncFromLag() {
    if (_disposed) return;
    final now = DateTime.now();
    final last = _lastLagResync;
    if (last != null && now.difference(last) < const Duration(seconds: 1)) {
      return;
    }
    _lastLagResync = now;
    debugLagResyncCount++;
    // Offline: nothing to send now; the reconnect path re-pulls from our cursor.
    if (_channel == null) return;
    _send(
      _doc == null
          ? {'type': 'sync.bootstrap'}
          : {'type': 'sync.pull', 'payload': _pullPayload()},
    );
  }

  /// Test-only: feed a raw server frame through the message handler.
  @visibleForTesting
  void debugHandleFrame(String raw) => _onMessage(raw);

  /// Apply the editor's op batch to the local replica and push the resulting
  /// CRDT diff to the cloud. The same op stream the offline backend consumes, so
  /// local and cloud editing behave identically.
  void applyLocalOps(List<DocOp> ops) {
    final doc = _doc;
    if (doc == null) return; // not bootstrapped yet
    final sv = doc.stateVector();
    for (final op in ops) {
      doc.applyEditorOp(op);
    }
    final diff = doc.encodeDiffSince(sv);
    if (diff.isEmpty) return;
    _enqueue(diff);
    _updateSyncPhase(); // a local edit → at least one unsynced diff now
  }

  /// Persist a local diff to the outbox and send it if we're connected.
  void _enqueue(Uint8List diff) {
    if (_useAppendLog) {
      // Durable append (survives restart/crash); its monotonic `clock` is the
      // push id the server echoes in `sync.ack`.
      final clock = persistence!.appendOutbox(diff);
      _maybeCompact(); // durable already — only bound the log, no snapshot churn
      // Durable regardless; only actively push if not stalled on a poison edit
      // (a permanently-rejected earlier clock — it would just pile up unacked).
      if (_channel != null && _ready && !_pushStalled) {
        _sendPushRaw(clock.toString(), diff);
        _sentThroughClock = clock;
      }
      return;
    }
    final p = _Pending('${_pushSeq++}', diff);
    _unacked.add(p);
    _persistSoon(); // legacy durability: the prefs unacked queue
    if (_channel != null && _ready) _sendPush(p);
  }

  /// Push un-acked diffs. [resendAll] re-sends the whole outbox (after a
  /// (re)connect, where in-flight pushes may have been lost); otherwise only the
  /// not-yet-sent ones go out, so a live drain doesn't spam duplicates.
  void _flushUnacked({bool resendAll = false}) {
    if (_channel == null || !_ready) return;
    if (_useAppendLog) {
      // While stalled on a poison edit, only a (re)connect's resendAll retries —
      // live/drain flushes don't re-flood the server (bounded by the budget).
      if (_pushStalled && !resendAll) return;
      // The un-pushed outbox is `clock > pushed_clock`; within a connection,
      // skip what we already sent (> _sentThroughClock). Idempotent regardless —
      // the server folds duplicate updates.
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

  /// Persist the full replica as the base snapshot NOW. P4-1: this runs only
  /// when a server base arrives (once per bootstrap/re-bootstrap) or as
  /// self-heal — steady-state persistence is pure append-log
  /// (appendOutbox/appendRemote, durable at the moment of the event), and
  /// [_maybeCompact] re-baselines periodically.
  void _saveBaseNow() {
    final doc = _doc;
    final store = persistence;
    if (doc == null || store == null) return;
    store.save(doc.encodeState(), _cursor);
  }

  int _appendsSinceCompactCheck = 0;
  static const int _compactCheckEvery = 32;
  static const int _compactThreshold = 256;

  /// Bound the logs without timers: every [_compactCheckEvery] appends, check
  /// the combined log size; past [_compactThreshold], fold base + logs into a
  /// fresh base (squash — clears the remote log and the acked outbox prefix;
  /// P2a keeps the un-pushed tail and the clock monotonic). Amortizes the full
  /// doc re-encode to ≤ once per [_compactCheckEvery] updates, instead of the
  /// old once-per-400ms-idle.
  void _maybeCompact() {
    final store = persistence;
    if (store == null) return;
    if (++_appendsSinceCompactCheck < _compactCheckEvery) return;
    _appendsSinceCompactCheck = 0;
    final sizes = store.logSizes();
    if (sizes.local + sizes.remote > _compactThreshold) _compactNow();
  }

  /// Fold + trim now (also the hard-close flush — a compacted store reopens
  /// fastest, and the fold is exactly what the old full-snapshot flush wrote).
  void _compactNow() {
    final store = persistence;
    if (store == null || _doc == null) return;
    store.compact();
  }

  /// True once a frame proves the socket is delivering; false when it drops.
  /// Feeds the sync-phase signal — distinct from [_ready] (about bootstrap) and
  /// the latch-once [_sawServerFrame].
  bool _online = false;
  SyncPhase? _lastSyncPhase;

  /// Recompute the derived sync phase and fire [onSyncPhase] only on a real
  /// change. `pending` is a boolean (outbox non-empty), never a count — no
  /// comparable product surfaces a number.
  void _updateSyncPhase() {
    final cb = onSyncPhase;
    if (cb == null) return;
    final pending = persistence != null
        ? persistence!.outboxAfter(persistence!.cursor().pushedClock).isNotEmpty
        : _unacked.isNotEmpty;
    final phase = deriveSyncPhase(online: _online, pending: pending);
    if (phase == _lastSyncPhase) return;
    _lastSyncPhase = phase;
    cb(phase);
  }

  void _send(Map<String, dynamic> message) {
    try {
      _channel?.sink.add(jsonEncode(message));
    } catch (_) {
      // The socket may be mid-drop / refused / not yet settled (e.g. an edit made
      // while offline before the connection resolves). Dropping the frame is safe:
      // the durable outbox re-pushes it on the next (re)connect. Never let a send
      // failure crash the session.
    }
  }

  void _onDone() {
    _channel = null;
    _online = false;
    _updateSyncPhase(); // link down → offline
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

  /// Fast, synchronous local-durability flush for a hard app-exit(0): write the
  /// 300ms-debounced state to the local store so quitting loses no edits. Skips
  /// compaction (a reopen-speed optimization, not correctness) and the socket
  /// (we're quitting; unacked edits sit in the local outbox and resend). Only
  /// the desktop exit(0) path calls it (window_setup_desktop) — on web the
  /// browser owns tab close and IndexedDB is already durable — but it must
  /// exist on every platform for the shared `_cloudSession?.flushForExit()`
  /// call site.
  void flushForExit() {
    if (_disposed) return;
    _persistNow();
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
    _compactNow(); // hard-close: fold base+logs so the store reopens fastest
    _disposed = true;
    _reconnectTimer?.cancel();
    _sub?.cancel();
    _channel?.sink.close();
    _channel = null;
    persistence?.dispose(); // release per-session store resources (locks/handles)
  }
}
