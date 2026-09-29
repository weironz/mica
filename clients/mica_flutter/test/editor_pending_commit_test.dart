// Regressions for the editor's pending-edit commit path (OPTIMIZATION_PLAN
// P1-06/07/08/09).
//
// All four were "the edit is in the document but never reaches storage", which
// is the one failure mode this editor's flush path exists to prevent. They are
// pinned through the OBSERVABLE channel — the op batches handed to `onOps` and
// the data the document ends up with — rather than by reading private state, so
// the tests stay meaningful if the internals are reshaped.
@TestOn('vm')
library;

import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorNode _para(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text);

/// A controller whose `onOps` records every batch and can be gated/failed by the
/// test through [gate] and [fail].
class _Recorder {
  _Recorder();

  final List<List<DocOp>> batches = [];

  /// When set, each commit awaits this until the test completes it — the seam
  /// that lets a second edit land while the first batch is still in flight.
  Completer<void>? gate;

  /// When true, the next commit throws (a failed appendOutbox, in production).
  bool fail = false;

  Future<void> call(List<DocOp> ops) async {
    batches.add(ops);
    final g = gate;
    if (g != null) await g.future;
    if (fail) throw StateError('outbox append failed');
  }

  /// Every `update_block` op ever sent, flattened.
  List<DocOp> get updates => [
    for (final b in batches)
      for (final op in b)
        if (op['type'] == 'update_block') op,
  ];

  /// The `text` of the LAST update op sent for [id] — what the server would end
  /// up holding.
  String? lastTextFor(String id) {
    final mine = updates.where((o) => o['block_id'] == id).toList();
    return mine.isEmpty ? null : mine.last['text'] as String?;
  }

  /// The `data.marks` of the LAST update op sent for [id], as JSON-ish text.
  String? lastMarksFor(String id) {
    final mine = updates.where((o) => o['block_id'] == id).toList();
    if (mine.isEmpty) return null;
    final data = mine.last['data'];
    if (data is! Map) return null;
    final marks = data['marks'];
    if (marks == null) return null;
    // Dart's Map.toString() does NOT quote keys ({start: 0, ...}), so a matcher
    // written against JSON-looking text would never match. Normalise to readable
    // text here and assert on that.
    return (marks as List)
        .map((m) => '${m['type']}@${m['start']}-${m['end']}')
        .join(',');
  }
}

/// Let the controller's internal future chain reach the recorder.
///
/// `_send` enqueues the commit with `_chain.then(...)`, so `onOps` runs on a
/// MICROTASK, not synchronously — a test that asserts immediately after calling
/// `flushPending()` is reading before the send happened.
Future<void> settle() => Future<void>.delayed(Duration.zero);

void main() {
  // ── P1-06 ────────────────────────────────────────────────────────────────

  test('an edit made while a batch is in flight is still sent afterwards', () async {
    final rec = _Recorder();
    final c = EditorController(rootBlockId: 'root', onOps: rec.call);
    c.load([_para('b1', '')]);
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));

    // Batch A: the user types "A", the debounce fires and the commit is sent.
    c.setFocusedText('A', 1, 1);
    rec.gate = Completer<void>();
    final first = c.flushPending();
    await settle();

    // The user keeps typing into the SAME block while A is in flight.
    c.setFocusedText('AB', 2, 2);
    expect(rec.lastTextFor('b1'), 'A', reason: 'batch A carried "A"');

    // A completes. The naive `removeAll(ids)` cleared the flag B had just set, so
    // B was skipped by the next debounce and never sent — the user saw their
    // typing in the document while storage held only "A", until a reconcile
    // replaced it. The generation check is what keeps B dirty.
    rec.gate!.complete();
    rec.gate = null;
    await first;

    await c.flushPending();
    expect(
      rec.lastTextFor('b1'),
      'AB',
      reason: 'the newer edit must reach storage after the in-flight batch lands',
    );
  });

  test('a batch that carries no NEW edit does not hold back a clean block', () async {
    // The other half of the generation rule: a block NOT touched during the
    // round-trip must still be cleared, or every flush would resend it forever.
    final rec = _Recorder();
    final c = EditorController(rootBlockId: 'root', onOps: rec.call);
    c.load([_para('b1', '')]);
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedText('A', 1, 1);

    rec.gate = Completer<void>();
    final first = c.flushPending();
    await settle();
    rec.gate!.complete();
    rec.gate = null;
    await first;

    final before = rec.batches.length;
    await c.flushPending();
    expect(
      rec.batches.length,
      before,
      reason: 'a committed block is no longer dirty, so a second flush is a no-op',
    );
  });

  // ── P1-07 ────────────────────────────────────────────────────────────────

  test('a failed commit keeps the edit pending and re-sends it without new input', () async {
    final rec = _Recorder()..fail = true;
    final c = EditorController(rootBlockId: 'root', onOps: rec.call);
    c.load([_para('b1', '')]);
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedText('kept', 4, 4);

    final ok = await c.flushPending();
    expect(ok, isFalse, reason: 'a failed commit reports failure');
    expect(c.opFaultCount, 1);
    // Count the ATTEMPTS, not the text. The failed attempt itself carried "kept",
    // so asserting on the last text sent cannot tell a retry from the original
    // failure — which is exactly how the first version of this test passed
    // against the buggy controller. A retry is a SECOND attempt at the backend.
    final attemptsAfterFailure = rec.batches.length;
    expect(attemptsAfterFailure, 1, reason: 'one attempt so far, and it failed');

    // Now stop touching the document entirely. The backend recovers, and the ONLY
    // thing that can save this edit is the controller retrying on its own: the old
    // code cleared the flag on failure and scheduled nothing, so no further
    // attempt ever happened and the typing died with the page switch.
    rec.fail = false;
    final deadline = DateTime.now().add(const Duration(seconds: 5));
    while (rec.batches.length == attemptsAfterFailure &&
        DateTime.now().isBefore(deadline)) {
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
    expect(
      rec.batches.length,
      greaterThan(attemptsAfterFailure),
      reason: 'the retained pending edit must be re-attempted automatically, with '
          'no further input from the user',
    );
    // The retry replays the SAME ops — that is the correct behaviour (the batch
    // never landed, so re-sending it is idempotent at the server), and asserting
    // anything else here would be asserting a different feature.
    expect(rec.lastTextFor('b1'), 'kept');
  });

  // ── P1-08 ────────────────────────────────────────────────────────────────

  test('an immediate structural send carries the dirty block\'s marks, not just its text', () async {
    final rec = _Recorder();
    final c = EditorController(rootBlockId: 'root', onOps: rec.call);
    c.load([
      EditorNode(id: 'b1', kind: 'paragraph', text: 'bold', data: {
        'marks': [
          {'start': 0, 'end': 4, 'type': 'bold'},
        ],
      }),
      _para('b2', 'second'),
    ]);
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));

    // A text edit re-offsets b1's marks and marks it dirty (debounced).
    c.setFocusedText('Xbold', 5, 5);
    // Then a STRUCTURAL op on a DIFFERENT block, within the debounce window:
    // this is `_sendNow`, which flushes b1 as a side effect.
    c.setSelection(const DocSelection.collapsed(DocPosition(1, 6)));
    c.splitAtCaret();
    await settle();

    final marks = rec.lastMarksFor('b1');
    expect(
      marks,
      contains('bold'),
      reason: 'the flushed block ships text AND data — its mark offsets were '
          're-derived for the new text, so sending text alone would persist a '
          'bold run pointing at the wrong characters',
    );
  });

  // ── P1-09 ────────────────────────────────────────────────────────────────

  test('reconcile keeps a dirty block\'s LOCAL marks alongside its local text', () async {
    final rec = _Recorder();
    final c = EditorController(rootBlockId: 'root', onOps: rec.call);
    c.load([
      EditorNode(id: 'b1', kind: 'paragraph', text: 'hi', data: {
        'marks': [
          {'start': 0, 'end': 2, 'type': 'bold'},
        ],
      }),
    ]);
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));

    // The user types, re-deriving b1's marks for the longer text. b1 is dirty.
    c.setFocusedText('hi!', 3, 3);

    // A server snapshot arrives carrying the OLD text and the OLD marks. Keeping
    // local text while adopting the server's offsets (the bug) pointed "bold" at
    // whatever sat at 0..2 of "hi!" — and that wrong pair was then committed.
    c.reconcile([
      EditorNode(id: 'b1', kind: 'paragraph', text: 'hi', data: {
        'marks': [
          {'start': 1, 'end': 2, 'type': 'bold'},
        ],
      }),
    ]);

    expect(c.nodes.first.text, 'hi!', reason: 'local text survives (pre-existing)');
    expect(
      c.nodes.first.data['marks'].toString(),
      contains('start: 0'),
      reason: 'local marks survive WITH the local text — the server\'s offsets '
          'described the server\'s text, not this one',
    );

    // And what gets committed is the local pair, consistently.
    await c.flushPending();
    await settle();
    expect(rec.lastTextFor('b1'), 'hi!');
    expect(rec.lastMarksFor('b1'), contains('bold@0-2'));
  });
}
