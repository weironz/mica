import 'dart:typed_data';

/// Local-first persistence for one cloud document (P2, option C — Phase 1).
///
/// A [CloudSyncSession] mirrors its yrs replica through this so the doc opens
/// from the on-device store — read offline across a restart, and resume the sync
/// stream from the saved cursor instead of always cold-bootstrapping. Deals in
/// raw yrs *state bytes* (not the FFI `MicaDocument`) so it is platform-agnostic;
/// desktop backs it with the SQLite `MicaStore`, web passes null (CanvasKit has
/// no on-device store yet — that stays online, gated by `kIsWeb`).
abstract class CloudDocStore {
  /// The persisted replica bytes + how far it had synced (the highest stream
  /// `rid` applied), or null if this doc has never been mirrored locally.
  ({Uint8List state, int cursor})? load();

  /// Persist the current replica bytes + synced cursor. The session debounces
  /// calls, so this can write straight through.
  void save(Uint8List state, int cursor);

  // ── outbox: durable un-pushed local edits (P2 offline edit) ────────────────
  //
  // Each local edit is appended here as a yrs diff keyed by a monotonic `clock`;
  // the un-pushed queue is `outboxAfter(cursor().pushedClock)`. This replaces the
  // prefs `cloudUnacked` queue with an append-log that survives a restart and a
  // crash mid-push (re-pushed idempotently, since yrs updates fold). Wired only
  // to the desktop store; the P2a additions have no caller yet.

  /// Append a local yrs `diff` to the outbox; returns its monotonic `clock`
  /// (strictly increasing for the doc's lifetime, even across [trimOutboxThrough]).
  int appendOutbox(Uint8List diff);

  /// Un-pushed outbox entries with `clock > pushedClock`, ordered — what to
  /// (re)send on connect. Pass `cursor().pushedClock`.
  List<({int clock, Uint8List bytes})> outboxAfter(int pushedClock);

  /// This doc's sync progress: the highest stream `rid` applied and the highest
  /// local `clock` the server has acked.
  ({int lastSyncedRid, int pushedClock}) cursor();

  /// Advance the sync cursor (only the passed fields; the rest keep their value).
  void advance({int? lastSyncedRid, int? pushedClock});

  /// Drop acked outbox entries (`clock ≤ pushedClock`) to bound the log; the
  /// un-pushed tail is kept and the clock stays monotonic.
  void trimOutboxThrough(int pushedClock);
}
