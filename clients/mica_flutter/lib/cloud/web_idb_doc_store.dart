// P4-2: the WEB local-first store — [CloudDocStore] over the browser's
// IndexedDB, giving web the same nbstore shape the desktop has in SQLite
// (base snapshot + local outbox log + remote log + sync cursor, per cloud doc).
//
// IndexedDB is async but [CloudDocStore] is synchronous (the desktop backs it
// with sync FFI, and the session logic depends on "durable before send").
// Bridge: [open] HYDRATES the doc's rows into memory once, then every mutation
// is served synchronously from the in-memory copy and mirrored to IndexedDB on
// a serialized write-behind queue. Web durability is therefore "flushed within
// the same event-loop turn-ish" rather than "fsync'd before return" — weaker
// than desktop, but the authority for cloud docs is the server; a lost tail is
// re-pulled/re-pushed on the next connect (same recovery story as a crash
// between desktop journal syncs under synchronous=NORMAL).
//
// Web-only, picked via the conditional import in `doc_store_platform.dart`.
// IndexedDB is a browser built-in; the bindings below are hand-rolled over
// `dart:js_interop` (in-house first — `dart:indexed_db` was removed from the
// SDK and `package:web` isn't a dependency), same style as yjs_interop.dart.
import 'dart:async';
import 'dart:js_interop';
import 'dart:typed_data';

import '../web/mica_ydoc.dart';
import 'cloud_doc_store.dart';

// ── minimal IndexedDB bindings ───────────────────────────────────────────────

@JS('indexedDB')
external JSObject? get _jsIndexedDb;

/// `navigator.locks` (Web Locks API) — used to enforce ONE writable store per
/// (origin, doc) across all tabs. Null on the rare browser without it (then we
/// fall back to best-effort, same as before this guard existed).
@JS('navigator.locks')
external JSObject? get _jsLockManager;

extension type _LockManager(JSObject _) implements JSObject {
  external JSPromise<JSAny?> request(
    String name,
    JSObject options,
    JSFunction callback,
  );
}

/// `navigator.storage` (StorageManager) — used to request PERSISTENT storage so
/// the durable outbox (unpushed offline edits) can't be silently evicted under
/// storage pressure. Null on the rare browser without it.
@JS('navigator.storage')
external JSObject? get _jsStorageManager;

extension type _StorageManager(JSObject _) implements JSObject {
  external JSPromise<JSBoolean> persist();
}

bool _persistRequested = false;

/// Ask the browser (once per session) to make this origin's storage persistent,
/// so a storage-pressure eviction can't wipe the IndexedDB durable outbox —
/// pushed content cold-bootstraps back, but UNPUSHED offline edits would vanish
/// silently. Best-effort and fire-and-forget: the browser grants on its own
/// heuristics (engagement / PWA install), a denial or a missing API is harmless,
/// and it must never block or fail opening the store.
void _requestPersistentStorage() {
  if (_persistRequested) return;
  _persistRequested = true;
  final sm = _jsStorageManager;
  if (sm == null) return;
  try {
    _StorageManager(sm).persist().toDart.then((_) {}, onError: (_) {});
  } catch (_) {}
}

extension type _IdbFactory(JSObject _) implements JSObject {
  external _IdbRequest open(String name, int version);
  external _IdbRequest deleteDatabase(String name);
}

extension type _IdbRequest(JSObject _) implements JSObject {
  external set onsuccess(JSFunction f);
  external set onerror(JSFunction f);
  external set onupgradeneeded(JSFunction f);
  external set onblocked(JSFunction f);
  external JSAny? get result;
}

extension type _IdbDatabase(JSObject _) implements JSObject {
  external JSObject createObjectStore(String name);
  external _IdbTransaction transaction(JSArray<JSString> storeNames, String mode);
  external _DomStringList get objectStoreNames;
  external void close();
}

extension type _DomStringList(JSObject _) implements JSObject {
  external bool contains(String s);
}

extension type _IdbTransaction(JSObject _) implements JSObject {
  external _IdbObjectStore objectStore(String name);
  external set oncomplete(JSFunction f);
  external set onerror(JSFunction f);
  external set onabort(JSFunction f);
}

extension type _IdbObjectStore(JSObject _) implements JSObject {
  @JS('get')
  external _IdbRequest getValue(JSAny key);
  external _IdbRequest put(JSAny? value, JSAny key);
}

/// Await one IndexedDB request → its (dartify'd) result.
Future<Object?> _requested(_IdbRequest r) {
  final c = Completer<Object?>();
  r.onsuccess = ((JSAny? _) {
    if (!c.isCompleted) c.complete(r.result.dartify());
  }).toJS;
  r.onerror = ((JSAny? _) {
    if (!c.isCompleted) c.completeError(StateError('indexeddb request failed'));
  }).toJS;
  return c.future;
}

/// Await a transaction reaching `complete` (or fail on error/abort).
Future<void> _completed(_IdbTransaction t) {
  final c = Completer<void>();
  t.oncomplete = ((JSAny? _) {
    if (!c.isCompleted) c.complete();
  }).toJS;
  void fail(JSAny? _) {
    if (!c.isCompleted) c.completeError(StateError('indexeddb txn failed'));
  }

  t.onerror = fail.toJS;
  t.onabort = fail.toJS;
  return c.future;
}

// ── the store ────────────────────────────────────────────────────────────────

/// Fold `base + updates` (in order) into final state bytes. Injectable so the
/// storage layer tests run without the yjs bundle (replay correctness is
/// covered by the desktop `load_doc` tests + the cross-engine wire-compat
/// suite; here the subject is IndexedDB durability).
typedef WebDocReplay = Uint8List Function(Uint8List base, List<Uint8List> updates);

Uint8List _yjsReplay(Uint8List base, List<Uint8List> updates) {
  final doc = MicaYDoc.fromState(base);
  for (final u in updates) {
    doc.applyUpdate(u);
  }
  return doc.encodeState();
}

class WebIdbDocStore implements CloudDocStore {
  WebIdbDocStore._(this._db, this._key, this._replay, this._releaseLock);

  final _IdbDatabase _db;

  /// Row key: `origin|docId` — one browser DB holds every origin's mirrors,
  /// namespaced the way the desktop store scopes workspaces by origin.
  final String _key;

  final WebDocReplay _replay;

  /// Completes to release this store's Web Lock (see [open]); null when locks
  /// are unavailable. [dispose] fires it so a later same-doc store can acquire.
  final Completer<void>? _releaseLock;

  static const _dbName = 'mica-localfirst';
  static const _storeNames = ['base', 'outbox', 'remote', 'cursor'];

  static JSArray<JSString> get _jsStoreNames =>
      [for (final s in _storeNames) s.toJS].toJS;

  // In-memory hydrated copy (authoritative for reads; IDB is the mirror).
  Uint8List? _base;
  final List<({int clock, Uint8List bytes})> _outbox = [];
  final List<({int rid, Uint8List bytes})> _remote = [];
  int _lastSyncedRid = 0;
  int _pushedClock = 0;

  /// Serialized write-behind tail — IndexedDB requests are issued in order, so
  /// the mirror converges to the in-memory state; a broken chain flips
  /// [_broken] and further appends report failure (the session self-heals by
  /// snapshotting the live doc, same as the desktop path).
  Future<void> _tail = Future.value();
  bool _broken = false;

  /// Open (creating/upgrading if needed) the browser store and hydrate this
  /// doc's rows. Returns null when IndexedDB is unavailable (private mode on
  /// some browsers) — the session then runs online-only, exactly as before.
  ///
  /// SINGLE-WRITER: acquires a Web Lock on the (origin, doc) so only ONE tab
  /// holds a writable mirror at a time. The desktop store is single-process by
  /// construction; the browser is not — two tabs each with a hydrated in-memory
  /// copy would blind-overwrite the whole `rows` record and silently drop the
  /// other's un-pushed outbox (the "outbox never silently dropped" P2
  /// invariant). A second tab fails to acquire → returns null → that tab runs
  /// online-only (its edits still push live; nothing to lose), exactly the
  /// private-mode fallback. [dispose] releases the lock. (Same single-writer
  /// discipline AFFiNE enforces via SharedWorker.)
  ///
  /// [replay]/[dbName] are test seams (fake fold fn, throwaway database).
  static Future<WebIdbDocStore?> open(
    String origin,
    String docId, {
    WebDocReplay? replay,
    String dbName = _dbName,
  }) async {
    // Before relying on IndexedDB durability, ask for persistent storage so the
    // unpushed outbox can't be evicted out from under us (best-effort, once).
    _requestPersistentStorage();
    final key = '$origin|$docId';
    Completer<void>? release;
    try {
      release = await _acquireLock('$dbName|$key');
      if (release == null) return null; // another tab owns the writable mirror
      final db = await _openDb(dbName, retryOnMissingStores: true);
      if (db == null) {
        release.complete();
        return null;
      }
      final store =
          WebIdbDocStore._(db, key, replay ?? _yjsReplay, release);
      await store._hydrate();
      return store;
    } catch (_) {
      release?.complete(); // never strand the lock on a failed open
      return null;
    }
  }

  /// Request the (origin, doc) Web Lock, held until the returned completer
  /// fires. Returns null when the lock is already held elsewhere (`ifAvailable`
  /// → the browser calls back with `null`). When the Web Locks API is absent,
  /// returns a dummy completer (best-effort, pre-guard behavior) rather than
  /// blocking local-first entirely.
  static Future<Completer<void>?> _acquireLock(String name) {
    final mgr = _jsLockManager;
    if (mgr == null) return Future.value(Completer<void>());
    final granted = Completer<Completer<void>?>();
    final held = Completer<void>();
    final callback = (JSAny? lock) {
      if (lock == null) {
        if (!granted.isCompleted) granted.complete(null); // not available
        return null;
      }
      if (!granted.isCompleted) granted.complete(held);
      return held.future.toJS; // hold the lock until `held` completes
    }.toJS;
    final options = {'ifAvailable': true}.jsify() as JSObject;
    // Fire-and-forget: the promise resolves when the lock is released; we track
    // grant via `granted`, release via the returned completer.
    _LockManager(mgr).request(name, options, callback);
    return granted.future;
  }

  static Future<_IdbDatabase?> _openDb(
    String dbName, {
    required bool retryOnMissingStores,
  }) async {
    final factory = _jsIndexedDb;
    if (factory == null) return null;
    final req = _IdbFactory(factory).open(dbName, 1);
    req.onupgradeneeded = ((JSAny? _) {
      final d = _IdbDatabase(req.result! as JSObject);
      for (final s in _storeNames) {
        if (!d.objectStoreNames.contains(s)) {
          d.createObjectStore(s);
        }
      }
    }).toJS;
    final c = Completer<_IdbDatabase>();
    req.onsuccess = ((JSAny? _) {
      if (!c.isCompleted) c.complete(_IdbDatabase(req.result! as JSObject));
    }).toJS;
    void fail(JSAny? _) {
      if (!c.isCompleted) c.completeError(StateError('indexeddb open failed'));
    }

    req.onerror = fail.toJS;
    req.onblocked = fail.toJS;
    final db = await c.future;
    // A same-version DB that predates (or lost) our object stores never fires
    // onupgradeneeded, and every later transaction would throw — detect and
    // rebuild ONCE (its only content would be rows we can't reach anyway).
    final missing = _storeNames.any((s) => !db.objectStoreNames.contains(s));
    if (!missing) return db;
    db.close();
    if (!retryOnMissingStores) return null;
    await _deleteDb(dbName);
    return _openDb(dbName, retryOnMissingStores: false);
  }

  static Future<void> _deleteDb(String dbName) {
    final c = Completer<void>();
    final req = _IdbFactory(_jsIndexedDb!).deleteDatabase(dbName);
    void done(JSAny? _) {
      if (!c.isCompleted) c.complete();
    }

    req.onsuccess = done.toJS;
    req.onerror = done.toJS;
    req.onblocked = done.toJS;
    return c.future;
  }

  Future<void> _hydrate() async {
    final txn = _db.transaction(_jsStoreNames, 'readonly');
    final key = _key.toJS;
    final baseRec = await _requested(txn.objectStore('base').getValue(key));
    if (baseRec is Map) {
      final state = baseRec['state'];
      if (state is List) _base = _bytes(state);
    }
    final cursorRec = await _requested(txn.objectStore('cursor').getValue(key));
    if (cursorRec is Map) {
      _lastSyncedRid = (cursorRec['lastSyncedRid'] as num?)?.toInt() ?? 0;
      _pushedClock = (cursorRec['pushedClock'] as num?)?.toInt() ?? 0;
    }
    final outboxRec = await _requested(txn.objectStore('outbox').getValue(key));
    if (outboxRec is Map) {
      final rows = outboxRec['rows'];
      if (rows is List) {
        for (final r in rows) {
          if (r is Map) {
            final clock = (r['clock'] as num?)?.toInt();
            final bytes = r['bytes'];
            if (clock != null && bytes is List) {
              _outbox.add((clock: clock, bytes: _bytes(bytes)));
            }
          }
        }
        _outbox.sort((a, b) => a.clock.compareTo(b.clock));
      }
    }
    final remoteRec = await _requested(txn.objectStore('remote').getValue(key));
    if (remoteRec is Map) {
      final rows = remoteRec['rows'];
      if (rows is List) {
        for (final r in rows) {
          if (r is Map) {
            final rid = (r['rid'] as num?)?.toInt();
            final bytes = r['bytes'];
            if (rid != null && bytes is List) {
              _remote.add((rid: rid, bytes: _bytes(bytes)));
            }
          }
        }
        _remote.sort((a, b) => a.rid.compareTo(b.rid));
      }
    }
  }

  /// A stored byte payload comes back from `dartify` as a typed list or a
  /// plain JS number array depending on the engine — normalize to Uint8List.
  static Uint8List _bytes(List<dynamic> raw) =>
      raw is Uint8List ? raw : Uint8List.fromList(raw.cast<num>().map((n) => n.toInt()).toList());

  // ── write-behind mirror ──────────────────────────────────────────────────
  //
  // Each store keeps ONE record per doc (`rows: [...]`) rather than a row per
  // update: IndexedDB keys can't express the desktop's composite PK cheaply,
  // and the whole set is already in memory — writing the doc's current list is
  // simpler and atomic per store. Sizes stay bounded by compaction.
  //
  // Writes are FIFO-serialized on [_tail], so the on-disk state is always a
  // prefix of the operation sequence. Each op that advances the cursor writes
  // the cursor in the SAME transaction as the data that justifies it
  // (appendRemote(Batch)/save co-write cursor + rows atomically; standalone
  // `advance` only moves to rids whose data an earlier queued write already
  // persisted) — so the cursor can never point past on-disk data (P4-1
  // invariant), even mid-stream. The one hole would be a queued write that
  // executes AFTER an earlier one failed: the earlier failure rolls its whole
  // transaction back (IDB atomicity), but a later already-queued write would
  // still run and could persist a cursor whose data just rolled back. So once
  // any write fails we FREEZE — every already-queued write becomes a no-op and
  // [_broken] blocks new ones. The mirror then sits at the last fully
  // consistent snapshot (safe: the server is authoritative; reconnect re-pulls
  // the delta), rather than risking a torn cursor-ahead-of-data state.

  void _mirror(void Function(_IdbTransaction txn) write) {
    if (_broken) return;
    _tail = _tail.then((_) async {
      if (_broken) return; // an earlier queued write failed → freeze consistently
      final txn = _db.transaction(_jsStoreNames, 'readwrite');
      final done = _completed(txn);
      write(txn);
      await done;
    }).catchError((_) {
      _broken = true; // freeze + further appends report failure → session self-heals
    });
  }

  void _put(_IdbTransaction txn, String store, Map<String, Object> record) {
    txn.objectStore(store).put(record.jsify(), _key.toJS);
  }

  // Uint8List crosses `jsify` + IndexedDB's structured clone as a typed array —
  // store it directly rather than exploding into a JS number array.
  Map<String, Object> _outboxRecord() => {
    'rows': [
      for (final e in _outbox) {'clock': e.clock, 'bytes': e.bytes},
    ],
  };

  Map<String, Object> _remoteRecord() => {
    'rows': [
      for (final e in _remote) {'rid': e.rid, 'bytes': e.bytes},
    ],
  };

  Map<String, Object> _cursorRecord() => {
    'lastSyncedRid': _lastSyncedRid,
    'pushedClock': _pushedClock,
  };

  /// Replay base + remote log + local log into final state bytes — the web
  /// mirror of the desktop's `load_doc` (yjs updates commute + are idempotent).
  /// A corrupt base makes the yjs replay THROW; swallow it into null so a bad
  /// mirror reads as "never mirrored" (the desktop contract — its FFI load
  /// returns null) instead of crashing the session's connect()/compact().
  Uint8List? _replayedState() {
    final base = _base;
    if (base == null) return null;
    try {
      return _replay(base, [
        for (final e in _remote) e.bytes,
        for (final e in _outbox) e.bytes,
      ]);
    } catch (_) {
      return null;
    }
  }

  // ── CloudDocStore ────────────────────────────────────────────────────────

  @override
  ({Uint8List state, int cursor})? load() {
    final state = _replayedState();
    if (state == null) return null;
    return (state: state, cursor: _lastSyncedRid);
  }

  @override
  void save(Uint8List state, int cursor) {
    _base = state;
    if (cursor > _lastSyncedRid) _lastSyncedRid = cursor;
    _mirror((txn) {
      _put(txn, 'base', {'state': state});
      _put(txn, 'cursor', _cursorRecord());
    });
  }

  @override
  int appendOutbox(Uint8List diff) {
    if (_broken) {
      throw StateError('web outbox append failed (IndexedDB unavailable)');
    }
    // Monotonic past pushedClock across trims — the P2a invariant.
    final maxClock = _outbox.isEmpty ? 0 : _outbox.last.clock;
    final clock = (maxClock > _pushedClock ? maxClock : _pushedClock) + 1;
    _outbox.add((clock: clock, bytes: diff));
    _mirror((txn) => _put(txn, 'outbox', _outboxRecord()));
    return clock;
  }

  @override
  List<({int clock, Uint8List bytes})> outboxAfter(int pushedClock) => [
    for (final e in _outbox)
      if (e.clock > pushedClock) (clock: e.clock, bytes: e.bytes),
  ];

  @override
  ({int lastSyncedRid, int pushedClock}) cursor() =>
      (lastSyncedRid: _lastSyncedRid, pushedClock: _pushedClock);

  @override
  void advance({int? lastSyncedRid, int? pushedClock}) {
    if (lastSyncedRid != null) _lastSyncedRid = lastSyncedRid;
    if (pushedClock != null) _pushedClock = pushedClock;
    _mirror((txn) => _put(txn, 'cursor', _cursorRecord()));
  }

  @override
  void trimOutboxThrough(int pushedClock) {
    // Clamp like the desktop primitive: never delete un-pushed entries.
    final upTo = pushedClock < _pushedClock ? pushedClock : _pushedClock;
    _outbox.removeWhere((e) => e.clock <= upTo);
    _mirror((txn) => _put(txn, 'outbox', _outboxRecord()));
  }

  @override
  bool appendRemote(int rid, Uint8List update) {
    if (_broken) return false;
    if (_remote.every((e) => e.rid != rid)) {
      _remote.add((rid: rid, bytes: update));
      _remote.sort((a, b) => a.rid.compareTo(b.rid));
    }
    if (rid > _lastSyncedRid) _lastSyncedRid = rid;
    _mirror((txn) {
      _put(txn, 'remote', _remoteRecord());
      _put(txn, 'cursor', _cursorRecord());
    });
    return true;
  }

  @override
  bool appendRemoteBatch(List<({int rid, Uint8List update})> items) {
    if (_broken) return false;
    for (final i in items) {
      if (_remote.every((e) => e.rid != i.rid)) {
        _remote.add((rid: i.rid, bytes: i.update));
      }
      if (i.rid > _lastSyncedRid) _lastSyncedRid = i.rid;
    }
    _remote.sort((a, b) => a.rid.compareTo(b.rid));
    _mirror((txn) {
      _put(txn, 'remote', _remoteRecord());
      _put(txn, 'cursor', _cursorRecord());
    });
    return true;
  }

  @override
  ({int local, int remote}) logSizes() =>
      (local: _outbox.length, remote: _remote.length);

  @override
  void compact() {
    final folded = _replayedState();
    if (folded == null) return;
    _base = folded;
    _remote.clear();
    _outbox.removeWhere((e) => e.clock <= _pushedClock);
    _mirror((txn) {
      _put(txn, 'base', {'state': folded});
      _put(txn, 'remote', _remoteRecord());
      _put(txn, 'outbox', _outboxRecord());
      _put(txn, 'cursor', _cursorRecord());
    });
  }

  /// Await the write-behind mirror (tests / graceful teardown).
  Future<void> flush() => _tail;

  bool _disposed = false;

  /// Release the single-writer Web Lock and close the IDB connection.
  /// Idempotent. Wired to the cloud session's dispose; the desktop store's
  /// [dispose] is a no-op (its MicaStore is shared and outlives any session).
  ///
  /// The lock is freed PROMPTLY (not behind [_tail]) so the same tab rebuilding
  /// a session for this doc — e.g. the B3 sync-retry — can immediately reacquire
  /// the writable mirror instead of falling back to online-only. Any queued
  /// write-behind transactions still complete against the open connection; the
  /// connection is closed once they drain. A new owner that starts writing in
  /// the gap is safe: IndexedDB serializes transactions across connections, and
  /// a disposing store's last writes are best-effort (its replacement rehydrates
  /// from whatever durably landed).
  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _releaseLock?.complete();
    _tail.whenComplete(() {
      try {
        _db.close();
      } catch (_) {}
    });
  }
}
