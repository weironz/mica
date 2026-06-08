// P2 §6 本地→云迁移 — the pure, platform-agnostic core (no FFI, no network), so
// it unit-tests headless. Orchestration (enumerate views, upload blobs, drive a
// CloudSyncSession) lives in main.dart; this module owns the two tricky, easy-to-
// get-wrong transforms:
//
//   (1) which image file_ids need reconciling — the local CAS keys blobs by
//       sha256, the cloud by UUID, so on migration we re-upload each sha-keyed
//       blob and rewrite the block's file_id to the returned UUID. A file_id
//       that is already a UUID (e.g. cached from a prior cloud import) is left
//       alone — re-uploading it would be wrong (we don't hold its bytes locally).
//
//   (2) replaying a local block tree as `insert_block` ops under the *cloud*
//       doc's root. This is migration strategy (c): the freshly-created cloud
//       doc already owns a committed `meta.root`; pushing the local doc's full
//       yrs state (which carries its own `meta.root`) would be a concurrent LWW
//       write on the same `meta` key → non-deterministic winner → the losing
//       root's subtree orphans. By replaying as ops onto the cloud root we never
//       touch `meta`, so there is no collision. (See docs/phase2-offline-crdt.md
//       §7.1.) The local root block itself is not inserted — its content is
//       carried onto the cloud root and its children re-parented there.
//
// A `DocOp` is the editor's coarse block op (see local/doc_ops.dart); here it's
// just `Map<String, dynamic>`, kept dependency-free on purpose.

/// A 64-hex sha256 — a local CAS content id. Cloud file ids are UUIDs.
final RegExp _sha256Re = RegExp(r'^[0-9a-f]{64}$');

/// Whether [id] looks like a local-CAS blob id (sha256) rather than a cloud UUID.
bool isLocalBlobId(String id) => _sha256Re.hasMatch(id);

/// The unique local-blob (sha256) file_ids referenced by image blocks in
/// [blocks], in first-seen order. Skips image blocks whose file_id is empty or
/// already a cloud UUID, and de-dups repeats — so each blob uploads once.
List<String> imageBlobIds(List<Map<String, dynamic>> blocks) {
  final seen = <String>{};
  final out = <String>[];
  for (final b in blocks) {
    if (b['type'] != 'image') continue;
    final data = b['data'];
    if (data is! Map) continue;
    final fid = data['file_id'];
    if (fid is! String || fid.isEmpty) continue;
    if (!isLocalBlobId(fid)) continue;
    if (seen.add(fid)) out.add(fid);
  }
  return out;
}

/// Build the op stream that re-creates [blocks] (a local document, flat in
/// tree/DFS order) under [cloudRootId].
///
/// - The local root block ([localRootId]) is not inserted; instead a leading
///   `update_block` carries its kind/text/data onto the cloud root, and its
///   direct children are re-parented onto the cloud root.
/// - Every other block is inserted under its (preserved) parent id with its
///   sibling index, in parent-before-child order (so the mirror never inserts a
///   child before its parent exists).
/// - Image `file_id`s are rewritten via [idMap] (sha256 → cloud UUID); ids
///   absent from the map are left as-is (resilient to a missing/dangling blob).
List<Map<String, dynamic>> buildMigrationOps({
  required List<Map<String, dynamic>> blocks,
  required String localRootId,
  required String cloudRootId,
  required Map<String, String> idMap,
}) {
  final byId = {for (final b in blocks) b['id'] as String: b};
  final ops = <Map<String, dynamic>>[];
  final root = byId[localRootId];
  if (root == null) return ops;

  // (1) carry the local root's content onto the seeded cloud root.
  ops.add(<String, dynamic>{
    'type': 'update_block',
    'block_id': cloudRootId,
    'kind': root['type'],
    'text': root['text'] ?? '',
    'data': _rewriteData(root, idMap),
  });

  // (2) DFS the local root's subtree, inserting each block under its parent
  //     (cloud root substituted for the local root), preserving sibling order.
  void walk(String parentLocalId, String parentTargetId) {
    final parent = byId[parentLocalId];
    final children = (parent?['children'] as List?)?.cast<String>() ?? const [];
    for (var i = 0; i < children.length; i++) {
      final childId = children[i];
      final block = byId[childId];
      if (block == null) continue;
      ops.add(<String, dynamic>{
        'type': 'insert_block',
        'parent_id': parentTargetId,
        'index': i,
        'block': <String, dynamic>{
          'id': childId,
          'type': block['type'],
          'text': block['text'] ?? '',
          'data': _rewriteData(block, idMap),
          // Children are added by their own inserts; start empty so the mirror
          // doesn't expect not-yet-inserted ids.
          'children': const <String>[],
        },
      });
      walk(childId, childId);
    }
  }

  walk(localRootId, cloudRootId);
  return ops;
}

/// A copy of [block]'s `data` map with the image `file_id` rewritten via [idMap]
/// when present. Inline marks and other props are preserved verbatim.
Map<String, dynamic> _rewriteData(
  Map<String, dynamic> block,
  Map<String, String> idMap,
) {
  final data = block['data'];
  if (data is! Map) return <String, dynamic>{};
  final out = Map<String, dynamic>.from(data.cast<String, dynamic>());
  if (block['type'] == 'image') {
    final fid = out['file_id'];
    if (fid is String && idMap.containsKey(fid)) out['file_id'] = idMap[fid];
  }
  return out;
}
