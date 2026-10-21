part of 'editor.dart';

/// The in-house editor: one editing surface for the whole document.
///
/// Drop-in replacement for the old per-block `TextField` editor. It owns a
/// single [Focus], a single OS text-input connection (so IME/Chinese input
/// works), one document-wide selection, and renders every node itself through
/// [DocumentSurface]. The caret travels the document as if it were one
/// continuous page — no block chrome, Word/Typora feel (see docs/architecture/editor.md).
/// Lets a host (e.g. the page outline) ask the editor to scroll a block into
/// view. The editor wires its scroll implementation into [_scroll] on init.
class EditorScrollHook {
  void Function(String blockId)? _scroll;
  void scrollToBlock(String blockId) => _scroll?.call(blockId);
}

/// A single heading in the live document outline (table of contents).
class OutlineEntry {
  const OutlineEntry({
    required this.id,
    required this.text,
    required this.level,
  });
  final String id;
  final String text;
  final int level;

  @override
  bool operator ==(Object other) =>
      other is OutlineEntry &&
      other.id == id &&
      other.text == text &&
      other.level == level;

  @override
  int get hashCode => Object.hash(id, text, level);
}

/// Live table-of-contents feed. The editor republishes the current headings (in
/// document order) on every model change; the host's outline panel listens and
/// rebuilds. Decouples the outline from the frozen bootstrap snapshot so it
/// tracks the user's own edits in real time (not just navigation / remote sync).
class EditorOutlineHook extends ChangeNotifier {
  List<OutlineEntry> _headings = const [];
  List<OutlineEntry> get headings => _headings;

  void publish(List<OutlineEntry> next) {
    if (_same(next)) return;
    _headings = List.unmodifiable(next);
    notifyListeners();
  }

  bool _same(List<OutlineEntry> next) {
    if (next.length != _headings.length) return false;
    for (var i = 0; i < next.length; i++) {
      if (next[i] != _headings[i]) return false;
    }
    return true;
  }
}

/// Lets the host open the editor's in-page find bar (Ctrl+F) even when focus is
/// outside the editor. The editor wires [_open] on init; a no-op otherwise.
class EditorFindHook {
  void Function()? _open;
  void open() => _open?.call();
}

/// Publishes the focused block's kind (+ heading level) to the host, so the
/// optional format toolbar can highlight the button for the current block type
/// — e.g. which heading level the caret sits in. Change-detected.
class EditorActiveBlockHook extends ChangeNotifier {
  String? _kind;
  int? _level;
  String? get kind => _kind;
  int? get level => _level;

  void publish(String? kind, int? level) {
    if (kind == _kind && level == _level) return;
    _kind = kind;
    _level = level;
    notifyListeners();
  }
}

/// Case-insensitive, non-overlapping occurrences of [query] across [texts] (one
/// entry per document node), in document order — the enumerator behind the
/// in-page find bar. Empty query → no matches. Pure; exposed for testing.
List<({int node, int start, int end})> findTextMatches(
  List<String> texts,
  String query,
) {
  final matches = <({int node, int start, int end})>[];
  if (query.isEmpty) return matches;
  final needle = query.toLowerCase();
  for (var i = 0; i < texts.length; i++) {
    final hay = texts[i].toLowerCase();
    var from = 0;
    while (from <= hay.length - needle.length) {
      final at = hay.indexOf(needle, from);
      if (at < 0) break;
      matches.add((node: i, start: at, end: at + query.length));
      from = at + query.length;
    }
  }
  return matches;
}

/// Lets the host's formatting toolbar drive editor commands (block converts,
/// inline marks, inserts, undo/redo) without owning the controller. The
/// editor registers itself while mounted; calls are no-ops otherwise.
class EditorCommandHook {
  void Function(String type)? _toggleMark;
  void Function(String kind, Map<String, dynamic> data)? _setBlock;
  void Function(String kind)? _insert; // 'divider' | 'table' | 'image'
  VoidCallback? _editLink;
  VoidCallback? _undo;
  VoidCallback? _redo;
  VoidCallback? _focusFirstLine;
  void Function(String text)? _insertTopParagraph;
  VoidCallback? _resetDiagramViews;
  Future<void> Function()? _flush;
  String Function()? _documentMarkdown;
  EditorAiSnapshot Function()? _captureAiSnapshot;
  Future<void> Function(EditorAiSnapshot, AiEditPreview, bool Function()?)?
  _applyAiEdits;

  void toggleMark(String type) => _toggleMark?.call(type);
  void setBlock(String kind, [Map<String, dynamic> data = const {}]) =>
      _setBlock?.call(kind, data);
  void insert(String kind) => _insert?.call(kind);
  void editLink() => _editLink?.call();
  void undo() => _undo?.call();
  void redo() => _redo?.call();

  /// Move the caret to the start of the first body line (ArrowDown from the
  /// page title).
  void focusFirstLine() => _focusFirstLine?.call();

  /// Insert a new paragraph at the very top (Enter in the page title pushes
  /// the body down; [text] is the title remainder after the caret).
  void insertTopParagraph(String text) => _insertTopParagraph?.call(text);

  /// Restore all diagram previews to their natural zoom/pan — the page host
  /// calls this when a click lands outside the editor canvas (page margins).
  void resetDiagramViews() => _resetDiagramViews?.call();

  /// Flush any debounced/pending text edits to the backend NOW. The host calls
  /// this before navigating away (page/workspace switch) so the last <=400ms of
  /// typing isn't dropped when the editor is torn down. Returns when the edits
  /// have been emitted (and, for the cloud path, the round-trip is in flight).
  Future<void> flush() => _flush?.call() ?? Future.value();

  /// Read the mounted editor's live Markdown without changing its selection.
  String? documentMarkdown() => _documentMarkdown?.call();

  EditorAiSnapshot? captureAiSnapshot() => _captureAiSnapshot?.call();

  Future<void> applyAiEdits(
    EditorAiSnapshot source,
    AiEditPreview preview, {
    bool Function()? canApply,
  }) async {
    final apply = _applyAiEdits;
    if (apply == null) throw const AiEditConflict();
    await apply(source, preview, canApply);
  }
}

/// Should ONE pasted block be woven into the caret's line as inline text
/// instead of landing as a block of its own?
///
/// A line's kind is a decision the user already made for it, and a paste must
/// not overrule it — copying an H2 into a sentence used to drop a second H2
/// under it. So a pasted block kind merges in whenever the target line still
/// holds text once the paste lands ([selFrom], [selTo] being the range the
/// paste replaces in a [targetLength]-long line).
///
/// A target left EMPTY carries no such decision: it takes the block path and
/// adopts the pasted kind, which is what keeps "paste a heading onto a blank
/// line" giving a heading. The exception is an emptied line that is NOT a
/// default paragraph — an empty heading or quote the user set deliberately —
/// which keeps its own kind and takes the pasted text inline.
///
/// A plain `paragraph` spec always merges: it has no kind to impose.
///
/// Same rule, same judgement, as AppFlowy's `pasteSingleLineNode` (empty
/// default paragraph → replace the block, else insert the delta) and AFFiNE's
/// `canMerge`; ProseMirror#231 argues the why.
bool pasteMergesInline({
  required String specKind,
  required String targetKind,
  required int targetLength,
  required int selFrom,
  required int selTo,
}) {
  if (!EditorNode.isInlineTextKind(specKind)) return false;
  if (specKind == 'paragraph') return true;
  final emptied = selTo - selFrom >= targetLength;
  return !emptied || targetKind != 'paragraph';
}

/// A linkable page for the `[[` picker.
class PageLinkTarget {
  const PageLinkTarget({required this.id, required this.title});
  final String id;
  final String title;
}

// Horizontal caret steps move by GRAPHEME, the unit Backspace/Delete already
// use. Stepping one UTF-16 unit put the caret between the two halves of a
// surrogate pair (any emoji): typing or deleting there left a lone surrogate in
// the text, which becomes U+FFFD once encoded for yrs.

/// The grapheme boundary just before [offset] (requires `offset > 0`).
int _prevGraphemeOffset(String text, int offset) =>
    text.substring(0, offset).characters.skipLast(1).string.length;

/// The grapheme boundary just after [offset] (requires `offset < text.length`).
int _nextGraphemeOffset(String text, int offset) =>
    offset + text.substring(offset).characters.first.length;

/// Every pointer kind the editor's tap/drag gestures answer — all but
/// `trackpad`. A precision touchpad's two-finger swipe arrives as PointerPanZoom
/// events of that kind, and a drag recognizer accepts those as a drag. A brisk
/// swipe's first report clears both the scroll view's slop and the selection
/// pan's in one event, the deeper editor is asked first, and the swipe drew a
/// selection instead of scrolling (2026-10-21). Clicks and press-and-drag on a
/// touchpad are `mouse` events, so selecting with it still works;
/// `test/trackpad_scroll_test.dart` pins the swipe.
const Set<PointerDeviceKind> _selectionDragDevices = {
  PointerDeviceKind.touch,
  PointerDeviceKind.mouse,
  PointerDeviceKind.stylus,
  PointerDeviceKind.invertedStylus,
  PointerDeviceKind.unknown,
};
