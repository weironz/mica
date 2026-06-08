// P2 §7 upstream blob differ — the offline-insert pending-upload queue + the
// content-addressed image-id rewrite, both pure (no FFI, no IO, web-safe) so they
// unit-test headless.
//
// Background: in cloud mode an image is normally uploaded synchronously and the
// block stores the returned cloud file id (a UUID). When offline, that upload
// fails. Instead of dropping the image, we land the bytes in the on-device CAS
// (sha256) and let the block reference that sha256 as a *placeholder* file_id —
// the image renders from the local CAS immediately and offline. The sha is then
// queued here so a later reconcile (when the doc is next open online) can upload
// the bytes, learn the cloud UUID, and rewrite the block's file_id sha256→UUID.
//
// This mirrors §6 migration's sha→UUID reconciliation (see workspace_migration
// .dart), but driven by an offline insert rather than a one-shot migration. A
// `DocOp` here is just `Map<String, dynamic>`, kept dependency-free on purpose.
import 'dart:convert';

/// One queued offline image upload: the CAS [sha] holding its bytes, plus the
/// (workspace, doc) it was inserted into so reconcile can locate the block(s).
/// [name] is the original file name, replayed to the cloud `complete` call.
typedef PendingUpload = ({
  String sha,
  String workspaceId,
  String docId,
  String name,
});

/// An in-memory set of pending offline image uploads, with JSON (de)serialization
/// for pref-backed persistence. De-dups on (sha, workspace, doc) so re-inserting
/// the same bytes into the same doc queues once.
class PendingUploads {
  PendingUploads([Iterable<PendingUpload>? initial])
      : _items = [...?initial];

  final List<PendingUpload> _items;

  /// All queued entries, in insertion order (read-only view).
  List<PendingUpload> get all => List.unmodifiable(_items);

  bool get isEmpty => _items.isEmpty;
  bool get isNotEmpty => _items.isNotEmpty;

  /// Entries for one document — the unit a single reconcile pass processes (the
  /// active cloud doc), since only one cloud doc is live at a time.
  List<PendingUpload> forDoc(String workspaceId, String docId) => [
        for (final e in _items)
          if (e.workspaceId == workspaceId && e.docId == docId) e,
      ];

  /// Add [entry], unless an identical (sha, workspace, doc) one is already
  /// queued. Returns true when it was newly added.
  bool add(PendingUpload entry) {
    if (_has(entry.workspaceId, entry.docId, entry.sha)) return false;
    _items.add(entry);
    return true;
  }

  /// Remove the entry for ([workspaceId], [docId], [sha]). Returns true when one
  /// was removed.
  bool remove(String workspaceId, String docId, String sha) {
    final before = _items.length;
    _items.removeWhere(
      (e) => e.sha == sha && e.workspaceId == workspaceId && e.docId == docId,
    );
    return _items.length != before;
  }

  bool _has(String workspaceId, String docId, String sha) => _items.any(
        (e) => e.sha == sha && e.workspaceId == workspaceId && e.docId == docId,
      );

  /// Serialize to a compact JSON string for prefs. Short keys keep it small.
  String toJson() => jsonEncode([
        for (final e in _items)
          {'s': e.sha, 'w': e.workspaceId, 'd': e.docId, 'n': e.name},
      ]);

  /// Parse a [toJson] payload, tolerating null/empty/corrupt input (→ empty
  /// queue) so a bad pref never blocks startup.
  static PendingUploads fromJson(String? raw) {
    if (raw == null || raw.isEmpty) return PendingUploads();
    try {
      final decoded = jsonDecode(raw);
      if (decoded is! List) return PendingUploads();
      return PendingUploads([
        for (final e in decoded)
          if (e is Map && e['s'] is String && e['w'] is String && e['d'] is String)
            (
              sha: e['s'] as String,
              workspaceId: e['w'] as String,
              docId: e['d'] as String,
              name: e['n'] is String ? e['n'] as String : '',
            ),
      ]);
    } catch (_) {
      return PendingUploads();
    }
  }
}

/// Build the `update_block` ops that rewrite every image block in [blocks] whose
/// `data.file_id` equals [fromId] to [toId], preserving the rest of `data`.
///
/// Content-addressed by design: we match on the placeholder file_id rather than a
/// remembered block id, so one queue entry reconciles every block that references
/// the same offline blob (e.g. the image pasted twice) and survives block id
/// churn. Mirrors workspace_migration's image rewrite, shaped as the editor's
/// `update_block` op so it replays through the same DocOpMirror/CRDT path.
List<Map<String, dynamic>> buildImageIdRewriteOps({
  required List<Map<String, dynamic>> blocks,
  required String fromId,
  required String toId,
}) {
  final ops = <Map<String, dynamic>>[];
  for (final b in blocks) {
    if (b['type'] != 'image') continue;
    final data = b['data'];
    if (data is! Map) continue;
    if (data['file_id'] != fromId) continue;
    final newData = Map<String, dynamic>.from(data.cast<String, dynamic>())
      ..['file_id'] = toId;
    ops.add(<String, dynamic>{
      'type': 'update_block',
      'block_id': b['id'],
      'data': newData,
    });
  }
  return ops;
}
