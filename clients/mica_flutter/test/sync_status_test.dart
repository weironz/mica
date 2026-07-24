import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/sync_status.dart';

// The three-state sync phase is a pure derivation from two booleans (socket
// live? any unsynced edits?). Pin every combination so the icon can never claim
// "synced" while the link is down, and disappears the moment we're caught up.

void main() {
  test('offline dominates — a down link is always offline, pending or not', () {
    expect(deriveSyncPhase(online: false, pending: true), SyncPhase.offline);
    expect(deriveSyncPhase(online: false, pending: false), SyncPhase.offline);
  });

  test('online + unsynced edits == syncing', () {
    expect(deriveSyncPhase(online: true, pending: true), SyncPhase.syncing);
  });

  test('online + caught up == synced (the icon then shows nothing)', () {
    expect(deriveSyncPhase(online: true, pending: false), SyncPhase.synced);
  });

  test('only the healthy caught-up state is synced', () {
    // Guards against a future refactor quietly widening "synced".
    for (final online in [true, false]) {
      for (final pending in [true, false]) {
        final phase = deriveSyncPhase(online: online, pending: pending);
        expect(phase == SyncPhase.synced, online && !pending,
            reason: 'online=$online pending=$pending');
      }
    }
  });
}
