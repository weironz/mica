// P4-2: the web IndexedDB doc store, exercised against a REAL browser
// IndexedDB (`flutter test --platform chrome`). Uses the injectable replay
// seam (byte-concat) so the yjs bundle isn't needed — replay correctness is
// covered by the desktop load_doc tests + the cross-engine wire-compat suite;
// the subject here is durability across a reopen (= page reload), hydration,
// the P2a/P2b clock invariants the sync session depends on, and the P4-2 review
// hardening (corrupt-base → null, single-writer lock, write-failure freeze).
@TestOn('browser')
library;

import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/web_idb_doc_store.dart';

/// Deterministic fake fold: base ++ updates in replay order — makes ordering
/// (remote log before outbox) and double-fold behavior visible as bytes.
Uint8List concatReplay(Uint8List base, List<Uint8List> updates) =>
    Uint8List.fromList([...base, for (final u in updates) ...u]);

/// A replay that THROWS — models a corrupt base the real yjs engine can't
/// decode (Y.applyUpdate raises, unlike a bad incremental update which it
/// skips). The store must swallow this into a null load, not propagate.
Uint8List throwingReplay(Uint8List base, List<Uint8List> updates) =>
    throw StateError('corrupt base');

Uint8List b(List<int> v) => Uint8List.fromList(v);

var _dbSeq = 0;

Future<WebIdbDocStore> openStore({
  String? db,
  String doc = 'doc-1',
  WebDocReplay? replay,
}) async {
  final store = await WebIdbDocStore.open(
    'https://cloud.example',
    doc,
    replay: replay ?? concatReplay,
    dbName: db ?? 'mica-test-${_dbSeq++}',
  );
  expect(store, isNotNull, reason: 'IndexedDB must be available in the test browser');
  return store!;
}

/// Model a page reload: the prior holder is gone, so its Web Lock is released.
/// Dispose it and reopen the same (db, doc), retrying until the lock frees.
Future<WebIdbDocStore> reopen(
  WebIdbDocStore prior, {
  required String db,
  String doc = 'doc-1',
  WebDocReplay? replay,
}) async {
  await prior.flush();
  prior.dispose();
  for (var attempt = 0; attempt < 50; attempt++) {
    final s = await WebIdbDocStore.open(
      'https://cloud.example',
      doc,
      replay: replay ?? concatReplay,
      dbName: db,
    );
    if (s != null) return s;
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
  fail('reopen never reacquired the lock');
}

void main() {
  test('empty store: no state, zero cursor, empty logs', () async {
    final s = await openStore();
    expect(s.load(), isNull);
    expect(s.cursor(), (lastSyncedRid: 0, pushedClock: 0));
    expect(s.outboxAfter(0), isEmpty);
    expect(s.logSizes(), (local: 0, remote: 0));
    s.dispose();
  });

  test('outbox: monotonic clocks, trim clamps to pushedClock, survives reopen',
      () async {
    final db = 'mica-test-${_dbSeq++}';
    final s = await openStore(db: db);
    expect(s.appendOutbox(b([1])), 1);
    expect(s.appendOutbox(b([2])), 2);
    expect(s.appendOutbox(b([3])), 3);
    s.advance(pushedClock: 2);
    // Clamp: asking to trim past pushedClock must not delete un-pushed entries.
    s.trimOutboxThrough(99);
    expect([for (final e in s.outboxAfter(0)) e.clock], [3]);
    // The clock stays monotonic past the trim (P2a invariant).
    expect(s.appendOutbox(b([4])), 4);

    // Reopen = page reload: everything hydrates back from IndexedDB.
    final r = await reopen(s, db: db);
    expect(r.cursor(), (lastSyncedRid: 0, pushedClock: 2));
    final tail = r.outboxAfter(r.cursor().pushedClock);
    expect([for (final e in tail) e.clock], [3, 4]);
    expect([for (final e in tail) e.bytes], [b([3]), b([4])]);
    // And the next clock continues past the hydrated maximum.
    expect(r.appendOutbox(b([5])), 5);
    r.dispose();
  });

  test('remote log: rid-idempotent, advances lastSyncedRid, batch, reopen',
      () async {
    final db = 'mica-test-${_dbSeq++}';
    final s = await openStore(db: db);
    expect(s.appendRemote(5, b([50])), isTrue);
    expect(s.appendRemote(5, b([99])), isTrue); // duplicate rid → ignored
    expect(s.cursor().lastSyncedRid, 5);
    expect(
      s.appendRemoteBatch([(rid: 7, update: b([70])), (rid: 6, update: b([60]))]),
      isTrue,
    );
    expect(s.cursor().lastSyncedRid, 7);
    expect(s.logSizes(), (local: 0, remote: 3));

    final r = await reopen(s, db: db);
    expect(r.cursor().lastSyncedRid, 7);
    expect(r.logSizes(), (local: 0, remote: 3));
    // Hydrated sorted by rid; the duplicate-rid payload is the FIRST write.
    r.save(b([0]), 7); // give it a base so load() replays
    final state = r.load();
    expect(state, isNotNull);
    expect(state!.state, b([0, 50, 60, 70]));
    r.dispose();
  });

  test('load replays base + remote log + outbox (in that order)', () async {
    final s = await openStore();
    s.save(b([0]), 9);
    s.appendRemote(1, b([1]));
    s.appendRemote(2, b([2]));
    s.appendOutbox(b([9]));
    final loaded = s.load();
    expect(loaded, isNotNull);
    expect(loaded!.state, b([0, 1, 2, 9]));
    expect(loaded.cursor, 9);
    s.dispose();
  });

  test('compact folds into base, clears remote, keeps un-pushed tail', () async {
    final db = 'mica-test-${_dbSeq++}';
    final s = await openStore(db: db);
    s.save(b([0]), 3);
    expect(s.appendOutbox(b([10])), 1);
    expect(s.appendOutbox(b([20])), 2);
    s.appendRemote(1, b([1]));
    s.advance(pushedClock: 1);
    s.compact();
    expect(s.logSizes(), (local: 1, remote: 0));
    // Folded base = old base + remote + WHOLE outbox (mirrors desktop squash —
    // safe because real yjs updates are idempotent; the concat fake makes the
    // deliberate double-fold of the kept tail visible).
    final loaded = s.load();
    expect(loaded!.state, b([0, 1, 10, 20, 20]));

    // The compacted shape survives a reopen.
    final r = await reopen(s, db: db);
    expect(r.logSizes(), (local: 1, remote: 0));
    expect(r.load()!.state, b([0, 1, 10, 20, 20]));
    expect(r.cursor(), (lastSyncedRid: 3, pushedClock: 1));
    // Clock continuity after a compaction + reload.
    expect(r.appendOutbox(b([30])), 3);
    r.dispose();
  });

  test('two docs in one DB do not bleed into each other', () async {
    final db = 'mica-test-${_dbSeq++}';
    final a = await openStore(db: db, doc: 'doc-a');
    final c = await openStore(db: db, doc: 'doc-b');
    a.appendOutbox(b([1]));
    c.appendRemote(4, b([4]));
    final a2 = await reopen(a, db: db, doc: 'doc-a');
    final c2 = await reopen(c, db: db, doc: 'doc-b');
    expect(a2.logSizes(), (local: 1, remote: 0));
    expect(a2.cursor().lastSyncedRid, 0);
    expect(c2.logSizes(), (local: 0, remote: 1));
    expect(c2.cursor().lastSyncedRid, 4);
    a2.dispose();
    c2.dispose();
  });

  // ── P4-2 review hardening ──────────────────────────────────────────────────

  test('corrupt base → load() returns null (never throws), matching desktop',
      () async {
    // A base whose replay throws (real yjs Y.applyUpdate on a corrupt base).
    // Contract parity with the FFI store: load() must return null so the
    // session cold-bootstraps, not propagate and kill connect()/compact().
    final db = 'mica-test-${_dbSeq++}';
    final s = await openStore(db: db, replay: throwingReplay);
    s.save(b([1, 2, 3]), 4); // a "base" the throwing replay will choke on
    expect(s.load(), isNull, reason: 'corrupt base swallowed into null');
    expect(() => s.compact(), returnsNormally,
        reason: 'compact on a corrupt base is a safe no-op, not a throw');
    s.dispose();
  });

  test('single-writer: a second open of the same (db,doc) returns null (lock held)',
      () async {
    final db = 'mica-test-${_dbSeq++}';
    final a = await openStore(db: db, doc: 'solo');
    // Second concurrent opener of the SAME doc must not get a writable mirror —
    // it would blind-overwrite a's rows and drop its un-pushed outbox.
    final second = await WebIdbDocStore.open(
      'https://cloud.example',
      'solo',
      replay: concatReplay,
      dbName: db,
    );
    expect(second, isNull,
        reason: 'the Web Lock is held by `a`; the 2nd tab runs online-only');
    // After a releases (tab closed / session disposed), a fresh open succeeds.
    final third = await reopen(a, db: db, doc: 'solo');
    expect(third, isNotNull);
    third.dispose();
  });
}
