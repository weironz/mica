/// Core document model for the in-house editor engine.
///
/// The engine edits an in-memory document that is the source of truth while the
/// user is typing; it is serialized to backend blocks via the operation list
/// (see [EditorController]). For Milestone 1 the document is a flat, ordered
/// list of text nodes under the document root — the same shape the backend
/// stores today. Nesting, tables, and void nodes arrive in later milestones,
/// so the types here are intentionally easy to extend (an `attributes`/`data`
/// map per node, string node kinds matching the backend).
library;

/// A single block-level node. `kind` matches the backend block `type`
/// (`paragraph`, `heading`, `bulleted_list`, `numbered_list`, `todo`, `quote`,
/// `code_block`). `data` mirrors the backend block `data` map (heading level,
/// todo checked, …). For Milestone 1 `text` is plain text; inline marks live in
/// `data.marks` and are layered on in a later milestone.
class EditorNode {
  EditorNode({
    required this.id,
    required this.kind,
    required this.text,
    Map<String, dynamic>? data,
  }) : data = data ?? <String, dynamic>{};

  final String id;
  String kind;
  String text;
  Map<String, dynamic> data;

  bool get isCode => kind == 'code_block';

  /// Void/atomic nodes hold no inline caret — the caret skips over them and
  /// clicks snap to the nearest text node. Tables are edited via cell overlays;
  /// dividers and images are non-text content.
  static bool isAtomicKind(String kind) =>
      kind == 'table' || kind == 'divider' || kind == 'image';
  bool get isAtomic => isAtomicKind(kind);

  /// Nesting level for list/todo items (0 = top level), clamped for safety.
  int get indent => ((data['indent'] as num?)?.toInt() ?? 0).clamp(0, 8);

  bool get isListKind =>
      kind == 'bulleted_list' || kind == 'numbered_list' || kind == 'todo';

  int get headingLevel {
    final level = data['level'];
    if (level is int) return level.clamp(1, 6);
    if (level is num) return level.toInt().clamp(1, 6);
    return 1;
  }

  bool get todoChecked => data['checked'] == true;

  EditorNode copy() =>
      EditorNode(id: id, kind: kind, text: text, data: Map<String, dynamic>.from(data));
}

/// A caret location: the [node] index in the flat document and the character
/// [offset] within that node's text.
class DocPosition implements Comparable<DocPosition> {
  const DocPosition(this.node, this.offset);

  final int node;
  final int offset;

  DocPosition withOffset(int o) => DocPosition(node, o);

  @override
  int compareTo(DocPosition other) {
    if (node != other.node) return node.compareTo(other.node);
    return offset.compareTo(other.offset);
  }

  @override
  bool operator ==(Object other) =>
      other is DocPosition && other.node == node && other.offset == offset;

  @override
  int get hashCode => Object.hash(node, offset);

  @override
  String toString() => 'DocPosition($node, $offset)';
}

/// A selection range from [anchor] (where the drag/extend started) to [focus]
/// (the moving end / caret). A collapsed selection (anchor == focus) is the
/// caret. The document owns one selection that may span multiple nodes.
class DocSelection {
  const DocSelection({required this.anchor, required this.focus});

  const DocSelection.collapsed(DocPosition at) : anchor = at, focus = at;

  final DocPosition anchor;
  final DocPosition focus;

  bool get isCollapsed => anchor == focus;

  /// The earlier of anchor/focus in document order.
  DocPosition get start => anchor.compareTo(focus) <= 0 ? anchor : focus;

  /// The later of anchor/focus in document order.
  DocPosition get end => anchor.compareTo(focus) <= 0 ? focus : anchor;

  /// True when the selection covers more than one node.
  bool get isMultiNode => start.node != end.node;

  DocSelection collapseToFocus() => DocSelection.collapsed(focus);
  DocSelection collapseToStart() => DocSelection.collapsed(start);

  @override
  bool operator ==(Object other) =>
      other is DocSelection && other.anchor == anchor && other.focus == focus;

  @override
  int get hashCode => Object.hash(anchor, focus);

  @override
  String toString() => 'DocSelection($anchor -> $focus)';
}
