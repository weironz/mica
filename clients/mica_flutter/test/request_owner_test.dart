import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/session/request_owner.dart';

/// The owner an action was started for must survive its awaits, and a commit
/// point must refuse once the live owner differs
/// (docs/dev/async-review-checklist.md).
void main() {
  test('the captured owner survives awaits; a changed owner is stale', () async {
    var live = 'account-a';
    final gate = Completer<void>();
    final run = runOwned(live, () async {
      await gate.future;
      await Future<void>.delayed(Duration.zero);
      ensureOwner(live);
      return 'committed';
    });
    live = 'account-b';
    gate.complete();
    await expectLater(run, throwsA(isA<StaleResult>()));
  });

  test('an unchanged owner commits', () async {
    const live = 'account-a';
    final result = await runOwned(live, () async {
      await Future<void>.delayed(Duration.zero);
      ensureOwner(live);
      return 'committed';
    });
    expect(result, 'committed');
  });

  test('a workspace switch is stale for workspace-scoped results', () async {
    final run = runOwned('a', () async {
      await Future<void>.delayed(Duration.zero);
      ensureOwner('a', workspaceId: 'w1', selectedWorkspaceId: 'w2');
    });
    await expectLater(run, throwsA(isA<StaleResult>()));
  });

  test('signed out at the start (null owner) is still an owner', () async {
    final run = runOwned(null, () async {
      ensureOwner('account-a');
    });
    await expectLater(run, throwsA(isA<StaleResult>()));
    expect(ownerChanged('x'), isFalse, reason: 'outside runOwned: no owner');
  });

  test('outside runOwned there is nothing to compare', () {
    expect(() => ensureOwner('anything'), returnsNormally);
  });
}
