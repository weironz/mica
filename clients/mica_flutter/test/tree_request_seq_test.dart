// Regression for OPTIMIZATION_PLAN P1-11: overlapping page-tree fetches for one
// workspace must not let the LAST-ARRIVING response win.
//
// The shape: a workspace switch is still loading when the tree-change socket
// bell fires, so two fetches for the same workspace are in flight. If the older
// one is answered second, it used to write its tree, its ETag AND its offline
// mirror over the fresh ones. The stale ETag is the durable half of that — the
// next cold start sends it, the server answers 304, and the client keeps a tree
// the server has moved past. The visible symptom is a page that shows as missing
// on a tree that looks perfectly normal.
//
// The rule is small enough to state, and this is the only place it can be
// tested: staging "two responses arrive out of order" against a real client and
// socket is possible, but relying on that ordering to happen in production is
// exactly what was broken.
@TestOn('vm')
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/api/tree_request_seq.dart';

void main() {
  test('an older request is no longer current once a newer one starts', () {
    final seq = TreeRequestSeq();

    final first = seq.claim('ws-1');
    expect(seq.isCurrent('ws-1', first), isTrue, reason: 'nothing newer yet');

    // The bell fires while the switch is still loading.
    final second = seq.claim('ws-1');
    expect(
      seq.isCurrent('ws-1', first),
      isFalse,
      reason: 'the older response must be discarded, even though it is the one '
          'that may arrive last',
    );
    expect(seq.isCurrent('ws-1', second), isTrue);
  });

  test('workspaces do not supersede each other', () {
    final seq = TreeRequestSeq();

    final a = seq.claim('ws-a');
    final b = seq.claim('ws-b');

    // Fetching B must not invalidate A's in-flight fetch: they are different
    // trees and both are correct answers to their own question. A single global
    // counter here would drop one of them for no reason.
    expect(seq.isCurrent('ws-a', a), isTrue);
    expect(seq.isCurrent('ws-b', b), isTrue);
  });

  test('the newest request is current after several overlapping claims', () {
    final seq = TreeRequestSeq();
    final claimed = [for (var i = 0; i < 4; i++) seq.claim('ws-1')];

    for (final old in claimed.take(3)) {
      expect(seq.isCurrent('ws-1', old), isFalse, reason: 'superseded');
    }
    expect(seq.isCurrent('ws-1', claimed.last), isTrue);
  });

  test('a claim on an untouched workspace is current', () {
    // The first fetch of a workspace must be allowed to commit — an off-by-one
    // here would make the very first load silently do nothing.
    final seq = TreeRequestSeq();
    final only = seq.claim('fresh');
    expect(seq.isCurrent('fresh', only), isTrue);
  });

  test('forget and clear drop the records without breaking later claims', () {
    final seq = TreeRequestSeq();
    final a = seq.claim('ws-1');
    seq.forget('ws-1');
    // Forgetting is only bookkeeping; numbers keep growing, so the pre-forget
    // claim stays superseded rather than becoming current again by accident.
    expect(seq.isCurrent('ws-1', a), isFalse);

    final after = seq.claim('ws-1');
    expect(seq.isCurrent('ws-1', after), isTrue);

    seq.claim('ws-2');
    seq.clear();
    expect(seq.isCurrent('ws-2', 1), isFalse, reason: 'cleared');
  });
}
