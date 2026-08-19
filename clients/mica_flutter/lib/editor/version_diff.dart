/// Block-level diff between two versions of a document, for the read-only
/// version-history preview.
///
/// Pure and UI-free on purpose: the version dialog lives inside
/// `main.dart`'s part file, where nothing is reachable from a test. The rule
/// that decides what turns green/amber/red belongs somewhere it can be pinned
/// down by a test, so it lives here.
library;

import 'model.dart';

/// One version's decoded content, as the server hands it over: the root block
/// id plus every block in tree order.
typedef VersionContent = ({
  String rootBlockId,
  List<Map<String, dynamic>> blocks,
});

/// The top-level blocks of a version, in document order (the root's direct
/// children — the flat shape the editor mounts).
///
/// Block nesting in this model is *data*, not tree shape (`data.indent`,
/// `data.quote`, `data.li`), so the root's children already are every block a
/// reader sees. Walking deeper would buy nothing here.
List<Map<String, dynamic>> topBlocks(VersionContent content) {
  final byId = {for (final b in content.blocks) (b['id'] as String): b};
  final childIds =
      ((byId[content.rootBlockId]?['children'] as List?) ?? const [])
          .cast<String>();
  return [
    for (final id in childIds)
      if (byId[id] != null) byId[id]!,
  ];
}

/// Key-order-insensitive encoding of a block's `data`, so two blocks that carry
/// the same map written in a different order do not read as "changed".
/// `jsonEncode` alone would call that a difference, which is how a diff turns
/// into a wall of amber that means nothing.
String _canonical(Object? value) {
  if (value is Map) {
    final keys = value.keys.map((k) => k.toString()).toList()..sort();
    return '{${keys.map((k) => '$k:${_canonical(value[k])}').join(',')}}';
  }
  if (value is List) {
    return '[${value.map(_canonical).join(',')}]';
  }
  return '$value';
}

/// Two blocks are equal when kind + text + data all match.
bool blockEqual(Map<String, dynamic> a, Map<String, dynamic> b) {
  return a['type'] == b['type'] &&
      (a['text'] ?? '') == (b['text'] ?? '') &&
      _canonical(a['data']) == _canonical(b['data']);
}

EditorNode _toNode(Map<String, dynamic> b, String? diff) => EditorNode(
  id: b['id'] as String,
  kind: b['type'] as String? ?? 'paragraph',
  text: b['text'] as String? ?? '',
  data: Map<String, dynamic>.from((b['data'] as Map?) ?? const {}),
  diffStatus: diff,
);

/// Read-only editor nodes for [content], tagged with a block-level diff against
/// [previous]: `added` (here, not before), `changed` (same id, different
/// content), `deleted` (in [previous], gone now — spliced back in at its old
/// position as a struck-through ghost). No predecessor → plain nodes, no tint.
List<EditorNode> versionDiffNodes(
  VersionContent content,
  VersionContent? previous,
) {
  final current = topBlocks(content);
  if (previous == null) {
    return [for (final b in current) _toNode(b, null)];
  }
  final prevBlocks = topBlocks(previous);
  final prevById = {for (final b in prevBlocks) (b['id'] as String): b};
  final currentIds = {for (final b in current) b['id'] as String};

  // Group deleted blocks (in prev, not in current) by the surviving block they
  // follow, so they render at roughly their old position ('' = before all).
  final deletedAfter = <String, List<Map<String, dynamic>>>{};
  var lastSurviving = '';
  for (final p in prevBlocks) {
    final pid = p['id'] as String;
    if (currentIds.contains(pid)) {
      lastSurviving = pid;
    } else {
      (deletedAfter[lastSurviving] ??= []).add(p);
    }
  }

  final nodes = <EditorNode>[];
  for (final d in deletedAfter[''] ?? const []) {
    nodes.add(_toNode(d, 'deleted'));
  }
  for (final b in current) {
    final id = b['id'] as String;
    final before = prevById[id];
    final status = before == null
        ? 'added'
        : (blockEqual(b, before) ? null : 'changed');
    nodes.add(_toNode(b, status));
    for (final d in deletedAfter[id] ?? const []) {
      nodes.add(_toNode(d, 'deleted'));
    }
  }
  return nodes;
}
