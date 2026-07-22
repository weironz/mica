// Regression: a failed op commit must be surfaced, not silently swallowed.
//
// The editor controller's single send choke point (`_send`) used to end its
// chain with a bare `.catchError((_) {})`. That swallowed every commit failure,
// including the StoreCloudDocStore.appendOutbox StateError that is thrown ON
// PURPOSE so a dropped-from-outbox edit can't pass unnoticed (red line #1). This
// guards that the failure is now counted and reported through `onOpFault`.
@TestOn('vm')
library;

import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

EditorNode _para(String id, String text) =>
    EditorNode(id: id, kind: 'paragraph', text: text);

void main() {
  test('a failed op commit is surfaced via onOpFault, not swallowed', () async {
    var faultCount = 0;
    Object? lastError;
    final c = EditorController(
      rootBlockId: 'root',
      onOps: (_) async => throw StateError('outbox append failed'),
      onOpFault: (error, count) {
        faultCount = count;
        lastError = error;
      },
    );
    c.load([_para('b1', '')]);

    // Type into b1 (debounced, marks it dirty), then force the commit.
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedText('typed', 5, 5);
    await c.flushPending();

    expect(c.opFaultCount, 1, reason: 'the failed commit is counted');
    expect(faultCount, 1, reason: 'onOpFault fired with the running count');
    expect(lastError, isA<StateError>(),
        reason: 'the original error is carried through, not discarded');
  });

  test('a successful commit does not report a fault', () async {
    var faults = 0;
    final c = EditorController(
      rootBlockId: 'root',
      onOps: (_) async {},
      onOpFault: (_, count) => faults = count,
    );
    c.load([_para('b1', '')]);
    c.setSelection(const DocSelection.collapsed(DocPosition(0, 0)));
    c.setFocusedText('typed', 5, 5);
    await c.flushPending();

    expect(c.opFaultCount, 0);
    expect(faults, 0);
  });
}
