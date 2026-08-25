import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/cloud/body_ownership.dart';

/// The rule this pins is a data-loss red line, not a preference.
///
/// The REST bootstrap is the yrs base materialised into plain JSON: no CRDT
/// metadata, so assigning it REPLACES the body instead of merging. Do that on
/// top of a document a CRDT session is holding and the editor reconciles away
/// blocks the user just typed; their next keystroke diffs against the replaced
/// text and pushes a delete of their own writing, which `room.broadcast` never
/// echoes back. Nothing about it is visible until the text is gone.
void main() {
  const doc = '00000000-0000-4000-8000-000000000001';
  const other = '00000000-0000-4000-8000-000000000002';

  test('a snapshot for the document a session holds must NOT be applied', () {
    expect(
      restSnapshotMayReplaceBody(
        arrivingDocumentId: doc,
        crdtSessionDocumentId: doc,
      ),
      isFalse,
      reason: 'the session owns this body and merges the server itself',
    );
  });

  test('with no session, the snapshot is the only content there is', () {
    // The cold path: first ever open, or no on-device mirror. Refusing here
    // would leave the editor on the home pane forever.
    expect(
      restSnapshotMayReplaceBody(
        arrivingDocumentId: doc,
        crdtSessionDocumentId: null,
      ),
      isTrue,
    );
  });

  test('a session on a DIFFERENT document does not protect this one', () {
    // Switching workspaces leaves the previous document's session up for a
    // moment. Reading "some session exists" as "hands off" would strand the
    // newly opened page on the home pane.
    expect(
      restSnapshotMayReplaceBody(
        arrivingDocumentId: doc,
        crdtSessionDocumentId: other,
      ),
      isTrue,
    );
  });
}
