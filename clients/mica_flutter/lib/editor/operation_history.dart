import 'model.dart';

/// The authoritative CRDT history command, shared by desktop and web hosts.
enum HistoryAction { reset, undo, redo }

class EditorHistoryState {
  const EditorHistoryState({
    required this.nodes,
    required this.canUndo,
    required this.canRedo,
    this.undoDepth,
    this.redoDepth,
    this.persistenceError,
  });

  factory EditorHistoryState.fromBlocks(
    List<Map<String, dynamic>> blocks, {
    required bool canUndo,
    required bool canRedo,
    int? undoDepth,
    int? redoDepth,
    Object? persistenceError,
  }) => EditorHistoryState(
    nodes: [
      for (final b in blocks)
        EditorNode(
          id: b['id'] as String,
          kind: b['type'] as String,
          text: b['text'] as String? ?? '',
          data: Map<String, dynamic>.from(b['data'] as Map? ?? const {}),
        ),
    ],
    canUndo: canUndo,
    canRedo: canRedo,
    undoDepth: undoDepth,
    redoDepth: redoDepth,
    persistenceError: persistenceError,
  );

  final List<EditorNode> nodes;
  final bool canUndo;
  final bool canRedo;
  final int? undoDepth;
  final int? redoDepth;

  /// The command may already have changed the engine even when saving its diff
  /// failed. Apply this canonical state before reporting the persistence fault.
  final Object? persistenceError;
}

typedef ApplyHistory =
    Future<EditorHistoryState> Function(HistoryAction action);

/// Selection metadata is proportional to the two endpoints, not the document.
/// Block identities survive insertions and reorders; deleted blocks fall back
/// to a clamped index. Text offsets are clamped to the surviving block.
class HistorySelection {
  HistorySelection(List<EditorNode> nodes, DocSelection? selection)
    : _anchor = _endpoint(nodes, selection?.anchor),
      _focus = _endpoint(nodes, selection?.focus);

  final ({String id, int index, int offset})? _anchor;
  final ({String id, int index, int offset})? _focus;

  static ({String id, int index, int offset})? _endpoint(
    List<EditorNode> nodes,
    DocPosition? position,
  ) {
    if (position == null || nodes.isEmpty) return null;
    final index = position.node.clamp(0, nodes.length - 1);
    return (id: nodes[index].id, index: index, offset: position.offset);
  }

  DocSelection? restore(List<EditorNode> nodes) {
    if (nodes.isEmpty) return null;
    DocPosition resolve(({String id, int index, int offset})? endpoint) {
      if (endpoint == null) return const DocPosition(0, 0);
      var index = nodes.indexWhere((n) => n.id == endpoint.id);
      if (index < 0) index = endpoint.index.clamp(0, nodes.length - 1);
      return DocPosition(
        index,
        endpoint.offset.clamp(0, nodes[index].text.length),
      );
    }

    return DocSelection(anchor: resolve(_anchor), focus: resolve(_focus));
  }
}

/// Inverse journal for standalone editors with a custom block-op sink. Mica's
/// real local/cloud hosts use yrs/yjs history, which retains character identity
/// through remote edits. This fallback checks the expected value of each field
/// before reverting it, so a custom sink never overwrites a foreign value.
class OperationInverse {
  OperationInverse._(this._changes);
  final List<_InverseChange> _changes;
  bool get isEmpty => _changes.isEmpty;

  factory OperationInverse.capture(
    List<EditorNode> before,
    List<EditorNode> after,
    List<Map<String, dynamic>> ops,
  ) {
    final old = {for (final n in before) n.id: n};
    final next = {for (final n in after) n.id: n};
    final ids = <String>{};
    final moved = <String>{};
    for (final op in ops) {
      final id = op['block_id'] ?? (op['block'] as Map?)?['id'];
      if (id is String) {
        ids.add(id);
        if (op['type'] == 'move_block') moved.add(id);
      }
    }
    final changes = <_InverseChange>[];
    for (final id in ids) {
      final a = old[id];
      final b = next[id];
      if (a == null && b == null) continue;
      final index = before.indexWhere((n) => n.id == id);
      final predecessor = index > 0 ? before[index - 1].id : null;
      final successor = index >= 0 && index + 1 < before.length
          ? before[index + 1].id
          : null;
      if (a == null || b == null) {
        changes.add(
          _InverseChange(
            id: id,
            removed: a == null ? null : historyCopyNode(a),
            inserted: b == null ? null : historyCopyNode(b),
            predecessor: predecessor,
            successor: successor,
          ),
        );
        continue;
      }
      final prior = <String, dynamic>{};
      final expected = <String, dynamic>{};
      void field(String key, Object? from, Object? to) {
        if (!historyJsonEqual(from, to)) {
          prior[key] = _copyValue(from);
          expected[key] = _copyValue(to);
        }
      }

      field('kind', a.kind, b.kind);
      // Text and inline marks form one version, not independent properties.
      field('inline', [a.text, a.data['marks']], [b.text, b.data['marks']]);
      for (final key in {...a.data.keys, ...b.data.keys}..remove('marks')) {
        field('data:$key', a.data[key], b.data[key]);
      }
      final nextIndex = after.indexWhere((n) => n.id == id);
      if (prior.isNotEmpty || moved.contains(id)) {
        changes.add(
          _InverseChange(
            id: id,
            prior: prior,
            expected: expected,
            move: moved.contains(id),
            expectedPredecessor: nextIndex > 0 ? after[nextIndex - 1].id : null,
            expectedSuccessor: nextIndex >= 0 && nextIndex + 1 < after.length
                ? after[nextIndex + 1].id
                : null,
            predecessor: predecessor,
            successor: successor,
          ),
        );
      }
    }
    return OperationInverse._(changes);
  }

  List<EditorNode> apply(List<EditorNode> current) {
    final out = [for (final n in current) historyCopyNode(n)];
    for (final change in _changes.reversed) {
      var index = out.indexWhere((n) => n.id == change.id);
      if (change.inserted != null) {
        if (index >= 0 && _sameNode(out[index], change.inserted!)) {
          out.removeAt(index);
        }
        continue;
      }
      if (change.removed != null) {
        if (index < 0) {
          out.insert(change.position(out), historyCopyNode(change.removed!));
        }
        continue;
      }
      if (index < 0) continue;
      final node = out[index];
      for (final key in change.prior.keys) {
        final Object? value = switch (key) {
          'kind' => node.kind,
          'inline' => [node.text, node.data['marks']],
          _ => node.data[key.substring(5)],
        };
        if (!historyJsonEqual(value, change.expected[key])) continue;
        final prior = _copyValue(change.prior[key]);
        if (key == 'kind') {
          node.kind = prior as String;
        } else if (key == 'inline') {
          final inline = prior as List;
          node.text = inline[0] as String;
          if (inline[1] == null) {
            node.data.remove('marks');
          } else {
            node.data['marks'] = inline[1];
          }
        } else if (prior == null) {
          node.data.remove(key.substring(5));
        } else {
          node.data[key.substring(5)] = prior;
        }
      }
      if (change.move && change.isAtExpectedPosition(out, index)) {
        out.removeAt(index);
        out.insert(change.position(out), node);
      }
    }
    return out;
  }
}

class _InverseChange {
  _InverseChange({
    required this.id,
    this.prior = const {},
    this.expected = const {},
    this.removed,
    this.inserted,
    this.move = false,
    this.expectedPredecessor,
    this.expectedSuccessor,
    this.predecessor,
    this.successor,
  });
  final String id;
  final Map<String, dynamic> prior;
  final Map<String, dynamic> expected;
  final EditorNode? removed;
  final EditorNode? inserted;
  final bool move;
  final String? expectedPredecessor;
  final String? expectedSuccessor;
  final String? predecessor;
  final String? successor;

  bool isAtExpectedPosition(List<EditorNode> nodes, int index) =>
      (index > 0 ? nodes[index - 1].id : null) == expectedPredecessor &&
      (index + 1 < nodes.length ? nodes[index + 1].id : null) == expectedSuccessor;

  int position(List<EditorNode> nodes) {
    final after = nodes.indexWhere((n) => n.id == predecessor);
    if (after >= 0) return after + 1;
    final before = nodes.indexWhere((n) => n.id == successor);
    return before >= 0 ? before : 0;
  }
}

EditorNode historyCopyNode(EditorNode node) => EditorNode(
  id: node.id,
  kind: node.kind,
  text: node.text,
  data: Map<String, dynamic>.from(_copyValue(node.data) as Map),
  diffStatus: node.diffStatus,
);

bool _sameNode(EditorNode a, EditorNode b) =>
    a.kind == b.kind && a.text == b.text && historyJsonEqual(a.data, b.data);

Object? _copyValue(Object? value) => switch (value) {
  Map() => value.map((k, v) => MapEntry(k, _copyValue(v))),
  List() => value.map(_copyValue).toList(),
  _ => value,
};

bool historyJsonEqual(Object? a, Object? b) {
  if (identical(a, b)) return true;
  if (a is Map && b is Map) {
    return a.length == b.length &&
        a.keys.every((k) => b.containsKey(k) && historyJsonEqual(a[k], b[k]));
  }
  if (a is List && b is List) {
    return a.length == b.length &&
        List.generate(
          a.length,
          (i) => i,
        ).every((i) => historyJsonEqual(a[i], b[i]));
  }
  return a == b;
}
