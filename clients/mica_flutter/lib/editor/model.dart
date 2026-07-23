/// Core document model for the in-house editor engine.
///
/// The engine edits an in-memory document that is the source of truth while the
/// user is typing; it is serialized to backend blocks via the operation list
/// (see [EditorController]). The document is a flat, ordered list of block
/// nodes under the document root — the same shape the backend stores. Tables,
/// void (atomic) nodes, and list nesting have since landed on this shape, which
/// the types here were kept intentionally easy to extend for (an
/// `attributes`/`data` map per node, string node kinds matching the backend).
library;

/// The bundled monospace face for code (block + inline). The generic
/// `'monospace'` family does not resolve on Flutter web — CanvasKit falls
/// back to Roboto, a proportional face whose narrow spaces made code
/// indentation (YAML!) nearly invisible.
const String kMonoFont = 'RobotoMono';

/// A single block-level node. `kind` matches the backend block `type`
/// (`paragraph`, `heading`, `bulleted_list`, `numbered_list`, `todo`, `quote`,
/// `code_block`). `data` mirrors the backend block `data` map (heading level,
/// todo checked, …). `text` is the plain text; inline marks live in
/// `data.marks` (see `marks.dart`).
class EditorNode {
  EditorNode({
    required this.id,
    required this.kind,
    required this.text,
    Map<String, dynamic>? data,
    this.diffStatus,
  }) : data = data ?? <String, dynamic>{};

  final String id;
  String kind;
  String text;
  Map<String, dynamic> data;

  /// Version-preview diff tint (null in the live editor): 'added' | 'changed' |
  /// 'deleted'. The renderer paints a background band; 'deleted' also strikes
  /// through (a ghost of a block the version removed). See version-history diff.
  final String? diffStatus;

  bool get isCode => kind == 'code_block';

  /// Void/atomic nodes hold no inline caret — the caret skips over them and
  /// clicks snap to the nearest text node. Tables are edited via cell overlays;
  /// dividers and images are non-text content.
  static bool isAtomicKind(String kind) =>
      kind == 'table' || kind == 'divider' || kind == 'image' || kind == 'math_block';
  bool get isAtomic => isAtomicKind(kind);

  /// Edits as ONE unit even when the kind is textual: a diagram code block
  /// in its preview form. A selection touching it consumes the whole block —
  /// merging its source text into neighbors leaves stray code on the page.
  bool get isUnitBlock =>
      isAtomic ||
      (kind == 'code_block' &&
          data['language'] == 'mermaid' &&
          (data['view'] as String?) != 'code');

  /// Nesting level for list/todo items (0 = top level), clamped for safety.
  int get indent => ((data['indent'] as num?)?.toInt() ?? 0).clamp(0, 8);

  /// Quote nesting depth: the `quote` kind is depth ≥ 1; any other kind
  /// inside a blockquote carries `data.quote` (markdown import).
  int get quoteDepth {
    final d = ((data['quote'] as num?)?.toInt() ?? 0).clamp(0, 8);
    return kind == 'quote' && d < 1 ? 1 : d;
  }

  /// Container-child level inside a list item (`data.li`), or null when the
  /// block is not nested in an item.
  int? get liLevel {
    final v = data['li'];
    return v is num ? v.toInt().clamp(0, 8) : null;
  }

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

/// Character classes for word selection (double-click). Grouping classes
/// (CJK / alnum) expand over their whole run; the rest are per-character.
enum _CharClass { cjk, alnum, whitespace, other }

_CharClass _classOf(int code) {
  // Whitespace.
  if (code == 0x20 || code == 0x09 || code == 0x0A || code == 0x0D) {
    return _CharClass.whitespace;
  }
  // ASCII letters / digits / underscore → an identifier-ish run.
  final isDigit = code >= 0x30 && code <= 0x39;
  final isUpper = code >= 0x41 && code <= 0x5A;
  final isLower = code >= 0x61 && code <= 0x7A;
  if (isDigit || isUpper || isLower || code == 0x5F) return _CharClass.alnum;
  // CJK ideographs (Unified + Ext-A) and common Hiragana/Katakana/Hangul, plus
  // any non-ASCII letter-ish code point — treated as one grouping run so a
  // double-click in 中文 grabs the contiguous CJK string.
  if (code >= 0x3400 && code <= 0x9FFF) return _CharClass.cjk; // CJK ideographs
  if (code >= 0x3040 && code <= 0x30FF) return _CharClass.cjk; // kana
  if (code >= 0xAC00 && code <= 0xD7A3) return _CharClass.cjk; // hangul
  if (code >= 0xF900 && code <= 0xFAFF) return _CharClass.cjk; // compat ideographs
  if (code > 0x7F) return _CharClass.cjk; // other non-ASCII: group it
  return _CharClass.other;
}

/// The bounds of the "word" containing [offset] in [text] (for double-click
/// selection). Grouping classes (CJK / alphanumeric) expand over the whole run;
/// whitespace and punctuation are boundaries and select a single character.
/// Returns `(start, end)`; `start == end` means nothing selectable.
(int, int) wordBoundsAt(String text, int offset) {
  if (text.isEmpty) return (0, 0);
  // Look at the character to the right of the caret; at the very end, fall back
  // to the one on the left so a click past the last glyph still grabs it.
  final probe = offset < text.length ? offset : text.length - 1;
  final cls = _classOf(text.codeUnitAt(probe));
  if (cls == _CharClass.whitespace || cls == _CharClass.other) {
    return (probe, probe + 1); // boundary char: select just it
  }
  var start = probe;
  while (start > 0 && _classOf(text.codeUnitAt(start - 1)) == cls) {
    start--;
  }
  var end = probe + 1;
  while (end < text.length && _classOf(text.codeUnitAt(end)) == cls) {
    end++;
  }
  return (start, end);
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
