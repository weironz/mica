/// Who is allowed to write the open document's BODY.
///
/// A workspace switch has two sources for it, and they are not equal:
///
///  * the on-device CRDT replica, rendered instantly and then kept in step by
///    the yrs session — every server byte arrives as a merge, so a local edit
///    made in the meantime survives;
///  * the REST `bootstrap`, which is the yrs base **materialised into plain
///    JSON**. It carries no CRDT metadata, so it cannot merge — assigning it
///    replaces the whole body.
///
/// That replacement is not a cosmetic flicker. The editor reconciles to the
/// incoming block list and drops blocks it does not find; the user's next
/// keystroke then diffs against the replaced text and pushes a DELETE of what
/// they had just written — and `room.broadcast` excludes the sender, so no echo
/// brings it back. Silent, and permanent.
///
/// Hence the rule, in one place with its reasoning attached: **once a CRDT
/// session holds a document, the REST snapshot may no longer touch its body.**
/// The session pulls the server's state itself, as a merge.
bool restSnapshotMayReplaceBody({
  /// The document the REST snapshot that just arrived describes.
  required String arrivingDocumentId,

  /// The document a live CRDT session is holding, if any.
  required String? crdtSessionDocumentId,
}) {
  // No session, or it holds a different document: nothing on screen can carry
  // unsaved local edits for this one, so the snapshot is the best we have.
  // This is the cold path — first ever open, or no on-device mirror.
  return crdtSessionDocumentId != arrivingDocumentId;
}
