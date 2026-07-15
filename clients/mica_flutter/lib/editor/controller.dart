import 'dart:async';

import 'package:flutter/foundation.dart';

import 'highlight.dart' show kCodeLanguages, detectLanguage;
import 'marks.dart';
import 'model.dart';
import 'table.dart';

/// A change to the backend block document, in the JSON shape the API expects
/// (`insert_block` / `update_block` / `delete_block` / `move_block`).
typedef DocOp = Map<String, dynamic>;

/// Sends a batch of block operations to the backend and resolves when the new
/// snapshot has been applied. Wired to the app's REST `applyDocumentUpdate`.
typedef ApplyOps = Future<void> Function(List<DocOp> operations);

/// Owns the in-memory document and the single document-wide selection, applies
/// edits, and translates each edit into backend block operations.
///
/// This is the engine's source of truth while editing. Text edits are debounced
/// into `update_block` ops; structural edits (split/merge/insert/delete) flush
/// pending text first and send immediately. Remote snapshots are reconciled
/// without clobbering a node the user is actively editing or the caret.
class EditorController extends ChangeNotifier {
  EditorController({required this.rootBlockId, required this.onOps});

  final String rootBlockId;
  final ApplyOps onOps;

  final List<EditorNode> nodes = [];
  DocSelection? selection;

  /// Preferred x (in surface-local pixels) kept across consecutive Up/Down
  /// presses so the caret tracks a visual column. Cleared on any other move.
  double? goalX;

  static const Duration _debounce = Duration(milliseconds: 400);
  final Set<String> _dirty = {}; // node ids with unsent / in-flight text
  Timer? _saveTimer;
  Future<void> _chain = Future.value();
  int _idCounter = 0;

  // --- Undo / redo -----------------------------------------------------------
  // Snapshot history. Every committed change (the single `_send` choke point)
  // pushes the prior document onto [_undoStack] and clears [_redoStack].
  // [_present] mirrors the current committed document so a snapshot is captured
  // without re-walking the live nodes. Undo/redo restore a snapshot and emit the
  // block-op diff to bring the backend along. `_restoring` suppresses recording
  // while a restore is in flight.
  final List<_DocSnapshot> _undoStack = [];
  final List<_DocSnapshot> _redoStack = [];
  late _DocSnapshot _present = _snapshot();
  bool _restoring = false;
  static const int _maxHistory = 200;

  // ---------------------------------------------------------------------------
  // Loading & remote reconciliation
  // ---------------------------------------------------------------------------

  /// Replace the whole document (initial load). Does not emit ops.
  void load(List<EditorNode> next) {
    nodes
      ..clear()
      ..addAll(next.map((n) => n.copy()));
    _clampSelection();
    _undoStack.clear();
    _redoStack.clear();
    _present = _snapshot();
    notifyListeners();
  }

  /// Merge a server snapshot into the live document. Keeps the caret on the
  /// same node when possible and never overwrites the text of a node with
  /// unsent local edits.
  void reconcile(List<EditorNode> server) {
    final focusedId = (selection != null && selection!.focus.node < nodes.length)
        ? nodes[selection!.focus.node].id
        : null;
    final byId = {for (final n in nodes) n.id: n};
    final next = <EditorNode>[];
    for (final src in server) {
      final cur = byId[src.id];
      if (cur == null) {
        next.add(src.copy());
      } else {
        cur.kind = src.kind;
        cur.data = Map<String, dynamic>.from(src.data);
        if (!_dirty.contains(cur.id)) {
          cur.text = src.text;
        }
        next.add(cur);
      }
    }
    nodes
      ..clear()
      ..addAll(next);
    _remapSelection(focusedId);
    // The server snapshot is the new committed baseline; keep `_present` aligned
    // so the next change records a correct "before" state. History is preserved.
    _present = _snapshot();
    notifyListeners();
  }

  void _remapSelection(String? focusedId) {
    if (nodes.isEmpty) {
      selection = null;
      return;
    }
    if (focusedId != null) {
      final idx = nodes.indexWhere((n) => n.id == focusedId);
      if (idx >= 0) {
        final off = selection!.focus.offset.clamp(0, nodes[idx].text.length);
        selection = DocSelection.collapsed(DocPosition(idx, off));
        return;
      }
    }
    _clampSelection();
  }

  void _clampSelection() {
    if (nodes.isEmpty) {
      selection = null;
      return;
    }
    final sel = selection;
    if (sel == null) return;
    DocPosition clamp(DocPosition p) {
      final node = p.node.clamp(0, nodes.length - 1);
      final off = p.offset.clamp(0, nodes[node].text.length);
      return DocPosition(node, off);
    }

    selection = DocSelection(anchor: clamp(sel.anchor), focus: clamp(sel.focus));
  }

  // ---------------------------------------------------------------------------
  // Selection
  // ---------------------------------------------------------------------------

  void setSelection(DocSelection? sel, {bool keepGoalX = false}) {
    if (!keepGoalX) goalX = null;
    selection = sel;
    notifyListeners();
  }

  void collapseTo(DocPosition pos, {bool keepGoalX = false}) {
    setSelection(DocSelection.collapsed(pos), keepGoalX: keepGoalX);
  }

  EditorNode? get focusedNode {
    final sel = selection;
    if (sel == null || sel.focus.node >= nodes.length) return null;
    return nodes[sel.focus.node];
  }

  /// Double-click: select the "word" straddling [pos]. A word is a maximal run
  /// of like characters — CJK ideographs grouped together, ASCII letters/digits
  /// together — so a click in 中文 grabs the CJK run and a click in `foo_bar`
  /// grabs an identifier. Whitespace and punctuation are boundaries; clicking
  /// directly on one selects just that single character (Notion/browser feel).
  /// No-op on atomic blocks (they carry no inline text). Returns true on a select.
  bool selectWordAt(DocPosition pos) {
    if (pos.node < 0 || pos.node >= nodes.length) return false;
    final node = nodes[pos.node];
    if (node.isAtomic) return false;
    final text = node.text;
    if (text.isEmpty) return false;
    final (start, end) = wordBoundsAt(text, pos.offset.clamp(0, text.length));
    if (start == end) return false;
    setSelection(DocSelection(
      anchor: DocPosition(pos.node, start),
      focus: DocPosition(pos.node, end),
    ));
    return true;
  }

  /// Triple-click: select the whole text of block [index]. No-op on atomic
  /// blocks. Returns true on a select.
  bool selectBlockText(int index) {
    if (index < 0 || index >= nodes.length) return false;
    final node = nodes[index];
    if (node.isAtomic || node.text.isEmpty) return false;
    setSelection(DocSelection(
      anchor: DocPosition(index, 0),
      focus: DocPosition(index, node.text.length),
    ));
    return true;
  }

  // ---------------------------------------------------------------------------
  // Text editing (from IME / typing)
  // ---------------------------------------------------------------------------

  /// Replace the focused node's text and in-node selection (driven by the OS
  /// input connection). Schedules a debounced `update_block`.
  void setFocusedText(String text, int selStart, int selEnd) {
    final sel = selection;
    if (sel == null) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;
    final node = nodes[i];
    // Atomic blocks (math, image, divider, table) hold no caret-editable
    // text; a stale IME echo landing here must not clobber the block.
    if (node.isAtomic) return;
    final old = node.text;

    if (old != text) {
      final marks = marksFromData(node.data);
      if (marks.isNotEmpty) {
        var prefix = _commonPrefix(old, text);
        var oldEnd = old.length - _commonSuffix(old, text, prefix);
        // The naive diff is ambiguous when removed text repeats its neighbors
        // (deleting a linked "AA" before another "A" aligns on the wrong "A").
        // The edit must cover the previous ranged selection — widen to it so
        // marks die with their text instead of bleeding onto the neighbors.
        if (!sel.isCollapsed &&
            sel.start.node == i &&
            sel.end.node == i) {
          final selS = sel.start.offset.clamp(0, old.length);
          final selE = sel.end.offset.clamp(0, old.length);
          if (prefix > selS) prefix = selS;
          if (oldEnd < selE) oldEnd = selE;
        }
        final shifted = shiftMarks(
          marks,
          prefix,
          oldEnd,
          text.length - old.length,
          text.length,
        );
        node.data = {...node.data, 'marks': marksToJson(shifted)};
      }
    }

    node.text = text;
    selection = DocSelection(
      anchor: DocPosition(i, selStart.clamp(0, text.length)),
      focus: DocPosition(i, selEnd.clamp(0, text.length)),
    );
    goalX = null;
    _markDirty(node.id);
    notifyListeners();
  }

  /// Soft newline inside a code block with auto-indent: insert `\n` at [caret]
  /// (the offset just AFTER the break, in the post-newline text) followed by the
  /// leading whitespace of the line the break ends, so nested code keeps its
  /// column. The caret lands after the copied indent.
  void insertCodeNewline(int caret) {
    final sel = selection;
    if (sel == null) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;
    final node = nodes[i];
    if (!node.isCode) return;
    // [caret] indexes the text WITH the newline; the break sits at caret-1, so
    // the old (newline-free) text splits at the same offset.
    final at = (caret - 1).clamp(0, node.text.length);
    final before = node.text.substring(0, at);
    final after = node.text.substring(at);

    // First Enter, and the whole first line is a known language name: the
    // user typed ```yaml⏎ — the fence rule converted on the third backtick,
    // leaving "yaml" as text. Claim it as the block's language instead.
    final tag = before.trim().toLowerCase();
    if (after.isEmpty &&
        !before.contains('\n') &&
        node.data['language'] == null &&
        tag != 'auto' &&
        kCodeLanguages.contains(tag)) {
      node.text = '';
      node.data = {...node.data, 'language': tag};
      selection = DocSelection.collapsed(DocPosition(i, 0));
      goalX = null;
      _dirty.remove(node.id);
      _sendNow([
        {
          'type': 'update_block',
          'block_id': node.id,
          'text': '',
          'data': node.data,
        },
      ]);
      notifyListeners();
      return;
    }
    // Leading whitespace of the current line (the run after the last newline).
    final lineStart = before.lastIndexOf('\n') + 1;
    final line = before.substring(lineStart);
    final indent = line.substring(0, line.length - line.trimLeft().length);
    final text = '$before\n$indent$after';
    final pos = at + 1 + indent.length;
    node.text = text;
    selection = DocSelection.collapsed(DocPosition(i, pos));
    goalX = null;
    _markDirty(node.id);
    notifyListeners();
  }

  /// Toggle an inline mark over the current ranged (single-node) selection.
  /// For links, pass [href] to add; call without href on a linked range to remove.
  void toggleMark(String type, {String? href}) {
    final sel = selection;
    if (sel == null || sel.isCollapsed || sel.isMultiNode) return;
    final i = sel.focus.node;
    final node = nodes[i];
    if (node.kind == 'code_block' || node.kind == 'table') return;
    final from = sel.start.offset;
    final to = sel.end.offset;
    final marks = marksFromData(node.data);
    final has = rangeHasMark(marks, from, to, type);
    final add = !has;
    if (type == 'link' && add && (href == null || href.isEmpty)) return;
    final next = applyMark(marks, from, to, type, href: href, add: add);
    node.data = {...node.data, 'marks': marksToJson(next)};
    _dirty.remove(node.id);
    _sendNow([
      {
        'type': 'update_block',
        'block_id': node.id,
        'text': node.text,
        'data': node.data,
      },
    ]);
    notifyListeners();
  }

  int _commonPrefix(String a, String b) {
    final n = a.length < b.length ? a.length : b.length;
    var i = 0;
    while (i < n && a.codeUnitAt(i) == b.codeUnitAt(i)) {
      i++;
    }
    return i;
  }

  int _commonSuffix(String a, String b, int prefix) {
    final max = (a.length < b.length ? a.length : b.length) - prefix;
    var i = 0;
    while (i < max &&
        a.codeUnitAt(a.length - 1 - i) == b.codeUnitAt(b.length - 1 - i)) {
      i++;
    }
    return i;
  }

  void _markDirty(String id) {
    _dirty.add(id);
    _saveTimer?.cancel();
    _saveTimer = Timer(_debounce, flushPending);
  }

  /// Send any debounced text edits now.
  Future<void> flushPending() {
    _saveTimer?.cancel();
    _saveTimer = null;
    if (_dirty.isEmpty) return Future.value();
    final ids = _dirty.toList();
    final ops = <DocOp>[];
    for (final id in ids) {
      final node = nodes.where((n) => n.id == id).firstOrNull;
      if (node != null) {
        // Include data so shifted inline marks persist with the text.
        ops.add({
          'type': 'update_block',
          'block_id': id,
          'text': node.text,
          'data': node.data,
        });
      }
    }
    if (ops.isEmpty) {
      _dirty.clear();
      return Future.value();
    }
    final done = _send(ops);
    // Keep ids in `_dirty` until the round-trip completes so an interleaved
    // reconcile does not clobber the in-flight text.
    return done.whenComplete(() => _dirty.removeAll(ids));
  }

  // ---------------------------------------------------------------------------
  // Structural editing
  // ---------------------------------------------------------------------------

  /// Enter: split the focused node at the caret, or exit an empty list item.
  void splitAtCaret() {
    final sel = selection;
    if (sel == null || !sel.isCollapsed) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;
    final node = nodes[i];
    final at = sel.focus.offset.clamp(0, node.text.length);

    // Pressing Enter on an empty list/todo/quote item exits to a paragraph.
    if (node.text.isEmpty && _continuesOnEnter(node.kind)) {
      node.kind = 'paragraph';
      node.data = {};
      _sendNow([
        {'type': 'update_block', 'block_id': node.id, 'kind': 'paragraph', 'data': {}},
      ]);
      collapseTo(DocPosition(i, 0));
      return;
    }

    // Enter at the very start of a non-empty block: insert an empty
    // paragraph ABOVE and keep the block (kind + text + marks) intact below
    // — splitting would strand the format on the empty upper line.
    if (at == 0 && node.text.isNotEmpty) {
      final created = EditorNode(id: _genId(), kind: 'paragraph', text: '');
      nodes.insert(i, created);
      _sendNow([_insertOp(created, i)]);
      collapseTo(DocPosition(i + 1, 0));
      return;
    }

    final before = node.text.substring(0, at);
    final after = node.text.substring(at);
    final newKind = _continuesOnEnter(node.kind) ? node.kind : 'paragraph';
    final newData = _continuesOnEnter(node.kind)
        ? _continuationData(node)
        : <String, dynamic>{};
    // Marks split with the text: each half keeps (a rebased copy of) its own.
    final (beforeMarks, afterMarks) =
        splitMarks(marksFromData(node.data), at);
    if (afterMarks.isNotEmpty) newData['marks'] = marksToJson(afterMarks);
    final created = EditorNode(id: _genId(), kind: newKind, text: after, data: newData);

    node.text = before;
    node.data = {...node.data, 'marks': marksToJson(beforeMarks)};
    nodes.insert(i + 1, created);
    _dirty.remove(node.id);
    _sendNow([
      {
        'type': 'update_block',
        'block_id': node.id,
        'text': before,
        'data': node.data,
      },
      _insertOp(created, i + 1),
    ]);
    collapseTo(DocPosition(i + 1, 0));
  }

  /// Split driven by a newline in the OS editing value (Enter). [before]/[after]
  /// are the text on each side of the newline. An empty list item exits the list.
  void applyNewlineSplit(String before, String after) {
    final sel = selection;
    if (sel == null) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;
    final node = nodes[i];

    if (before.isEmpty && after.isEmpty && _continuesOnEnter(node.kind)) {
      node.kind = 'paragraph';
      node.data = {};
      node.text = '';
      _sendNow([
        {'type': 'update_block', 'block_id': node.id, 'kind': 'paragraph', 'data': {}, 'text': ''},
      ]);
      collapseTo(DocPosition(i, 0));
      return;
    }

    // Enter on the EMPTY FIRST LINE of a multi-line quote/list block (a
    // pasted quote split leaves the old soft break leading the new block):
    // that line leaves the group, Typora-style — it becomes a plain empty
    // paragraph ABOVE the remainder and the caret stays on it. Without this
    // the generic start-of-block branch below just stacks paragraphs above
    // while the caret sits forever on a barred empty line.
    if (before.isEmpty &&
        after.startsWith('\n') &&
        _continuesOnEnter(node.kind)) {
      final remainder = after.substring(1);
      final marks = shiftMarks(
        marksFromData(node.data),
        0,
        1,
        -1,
        remainder.length,
      );
      node.text = remainder;
      node.data = {...node.data, 'marks': marksToJson(marks)};
      final created = EditorNode(id: _genId(), kind: 'paragraph', text: '');
      nodes.insert(i, created);
      _dirty.remove(node.id);
      _sendNow([
        _insertOp(created, i),
        {
          'type': 'update_block',
          'block_id': node.id,
          'text': node.text,
          'data': node.data,
        },
      ]);
      collapseTo(DocPosition(i, 0));
      return;
    }

    // Enter at the start of a non-empty block (IME newline path): same
    // insert-paragraph-above behavior as splitAtCaret.
    if (before.isEmpty && after.isNotEmpty && after == node.text) {
      final created = EditorNode(id: _genId(), kind: 'paragraph', text: '');
      nodes.insert(i, created);
      _sendNow([_insertOp(created, i)]);
      collapseTo(DocPosition(i + 1, 0));
      return;
    }

    final continues = _continuesOnEnter(node.kind);
    final newData = continues ? _continuationData(node) : <String, dynamic>{};
    // Marks split with the text (split point approximated by the left half's
    // length — exact unless the same IME frame also changed the text).
    final (beforeMarks, afterMarks) = splitMarks(
      marksFromData(node.data),
      before.length.clamp(0, node.text.length),
    );
    if (afterMarks.isNotEmpty) newData['marks'] = marksToJson(afterMarks);
    final created = EditorNode(
      id: _genId(),
      kind: continues ? node.kind : 'paragraph',
      text: after,
      data: newData,
    );
    node.text = before;
    node.data = {...node.data, 'marks': marksToJson(beforeMarks)};
    nodes.insert(i + 1, created);
    _dirty.remove(node.id);
    _sendNow([
      {
        'type': 'update_block',
        'block_id': node.id,
        'text': before,
        'data': node.data,
      },
      _insertOp(created, i + 1),
    ]);
    collapseTo(DocPosition(i + 1, 0));
  }

  /// Backspace at offset 0: merge the focused node into the previous one.
  /// Returns false if there is no previous node (caller may fall through).
  bool mergeBackward() {
    final sel = selection;
    if (sel == null || !sel.isCollapsed) return false;
    final i = sel.focus.node;
    if (i < 0 || i >= nodes.length) return false;
    final cur = nodes[i];

    // The caret is parked ON an atomic block (a divider, or an image the user
    // clicked): Backspace removes it. This has to come first — an atomic block
    // holds no text by nature, so the "empty styled line falls back to a
    // paragraph" rule below would otherwise claim it and quietly turn the
    // picture into a blank line, wiping its file_id instead of deleting it.
    if (cur.isAtomic) {
      nodes.removeAt(i);
      _sendNow([
        {'type': 'delete_block', 'block_id': cur.id},
      ]);
      if (nodes.isEmpty) {
        ensureNotEmpty();
        return true;
      }
      // Land where the block was: end of the block above, else start of the
      // one that slid up into its place.
      final target = (i - 1).clamp(0, nodes.length - 1);
      collapseTo(DocPosition(
        target,
        i > 0 ? nodes[target].text.length : 0,
      ));
      return true;
    }

    // A styled-but-empty caret line first turns back into a plain paragraph.
    // This also lets Backspace clear an empty styled FIRST block (e.g. an empty
    // code block at the top), which has no previous block to merge into.
    if (cur.text.isEmpty && cur.kind != 'paragraph') {
      cur.kind = 'paragraph';
      cur.data = {};
      _sendNow([
        {'type': 'update_block', 'block_id': cur.id, 'kind': 'paragraph', 'data': {}},
      ]);
      notifyListeners();
      return true;
    }

    // A list item sheds its marker first — the bullet/number/checkbox is
    // visible format, and Backspace at the start is how you remove it
    // (Typora/Notion). Rising/merging only applies to the resulting
    // paragraph on the NEXT Backspace. (Nested items outdent in the key
    // handler before this runs.)
    if (cur.isListKind) {
      cur.kind = 'paragraph';
      cur.data = {...cur.data}
        ..remove('checked')
        ..remove('indent');
      _sendNow([
        {
          'type': 'update_block',
          'block_id': cur.id,
          'kind': 'paragraph',
          'data': cur.data,
        },
      ]);
      notifyListeners();
      return true;
    }

    // Nothing before the first block to merge into.
    if (i == 0) return false;
    final prev = nodes[i - 1];

    // An atomic neighbor (divider/table) can't absorb text — delete it instead
    // of merging, keeping the current node and its caret.
    if (prev.isAtomic) {
      nodes.removeAt(i - 1);
      _sendNow([
        {'type': 'delete_block', 'block_id': prev.id},
      ]);
      collapseTo(DocPosition(i - 1, 0));
      return true;
    }

    // An EMPTY line above is consumed whole: the current block rises one
    // line keeping its identity. (Merging would pour the block's text into
    // the empty paragraph's kind — one Backspace at a heading's start
    // silently destroyed the title.)
    if (prev.text.isEmpty) {
      nodes.removeAt(i - 1);
      _sendNow([
        {'type': 'delete_block', 'block_id': prev.id},
      ]);
      collapseTo(DocPosition(i - 1, 0));
      return true;
    }

    final junction = prev.text.length;
    final merged = prev.text + cur.text;
    prev.text = merged;
    prev.data = {
      ...prev.data,
      'marks': marksToJson(concatMarks(
        marksFromData(prev.data),
        marksFromData(cur.data),
        junction,
      )),
    };
    nodes.removeAt(i);
    _dirty
      ..remove(cur.id)
      ..remove(prev.id);
    _sendNow([
      {
        'type': 'update_block',
        'block_id': prev.id,
        'text': merged,
        'data': prev.data,
      },
      {'type': 'delete_block', 'block_id': cur.id},
    ]);
    collapseTo(DocPosition(i - 1, junction));
    return true;
  }

  /// Delete at end of node: merge the next node into the focused one.
  bool mergeForward() {
    final sel = selection;
    if (sel == null || !sel.isCollapsed) return false;
    final i = sel.focus.node;
    if (i < 0 || i >= nodes.length) return false;

    // The caret is parked ON an atomic block: Delete removes it, same as
    // Backspace. Without this the merge below would pull the FOLLOWING block's
    // text into the divider/image node — the block survives as a mongrel and
    // its neighbour disappears.
    final atCaret = nodes[i];
    if (atCaret.isAtomic) {
      nodes.removeAt(i);
      _sendNow([
        {'type': 'delete_block', 'block_id': atCaret.id},
      ]);
      if (nodes.isEmpty) {
        ensureNotEmpty();
        return true;
      }
      collapseTo(DocPosition(i.clamp(0, nodes.length - 1), 0));
      return true;
    }

    if (i + 1 >= nodes.length) return false;
    final cur = nodes[i];
    final next = nodes[i + 1];

    // Delete-at-end-of-line removes an atomic following node (divider/table)
    // rather than trying to merge its (empty) text in.
    if (next.isAtomic) {
      nodes.removeAt(i + 1);
      _sendNow([
        {'type': 'delete_block', 'block_id': next.id},
      ]);
      collapseTo(DocPosition(i, cur.text.length));
      return true;
    }

    final junction = cur.text.length;
    final merged = cur.text + next.text;
    cur.text = merged;
    cur.data = {
      ...cur.data,
      'marks': marksToJson(concatMarks(
        marksFromData(cur.data),
        marksFromData(next.data),
        junction,
      )),
    };
    nodes.removeAt(i + 1);
    _dirty
      ..remove(cur.id)
      ..remove(next.id);
    _sendNow([
      {
        'type': 'update_block',
        'block_id': cur.id,
        'text': merged,
        'data': cur.data,
      },
      {'type': 'delete_block', 'block_id': next.id},
    ]);
    collapseTo(DocPosition(i, junction));
    return true;
  }

  /// Serialize the current ranged selection to text (tables become GFM). Empty
  /// when the selection is collapsed/absent. [imageUrls] maps an image's
  /// `file_id` to a fresh download URL so copied Markdown links actually resolve
  /// (falling back to the external url, then the bare filename).
  String selectionText({Map<String, String>? imageUrls}) {
    final sel = selection;
    if (sel == null || sel.isCollapsed) return '';
    final s = sel.start;
    final e = sel.end;

    String nodeText(int i, int from, int to) {
      final node = nodes[i];
      if (node.kind == 'table') {
        return tableToMarkdown(TableData.fromBlock(node.data));
      }
      if (node.kind == 'image') {
        final fileId = node.data['file_id'] as String?;
        final target = (imageUrls?[fileId] ??
            node.data['url'] ??
            node.data['name'] ??
            '') as String;
        return '![${node.text}]($target)';
      }
      if (node.kind == 'divider') return '---';
      final len = node.text.length;
      final a = from.clamp(0, len);
      final b = to.clamp(0, len);
      final full = a == 0 && b == len; // whole block selected
      final sub = node.text.substring(a, b);
      if (node.kind == 'code_block') {
        if (!full) return sub;
        return '```${_copyLanguage(node)}\n$sub\n```';
      }
      final marks = <Mark>[];
      for (final m in marksFromData(node.data)) {
        final s = m.start.clamp(a, b);
        final e = m.end.clamp(a, b);
        if (e > s) marks.add(Mark(s - a, e - a, m.type, href: m.href));
      }
      var inline = marks.isEmpty ? sub : inlineToMarkdown(sub, marks);
      if (full && node.kind == 'paragraph') {
        // A copied paragraph that looks like a list/heading/divider must
        // not change kind when pasted back as markdown.
        inline = escapeBlockLeader(inline);
      }
      // Prepend the block-level Markdown marker only for a fully-selected block
      // (a partial first/last line is copied as plain inline text).
      return full ? '${_blockPrefix(node)}$inline' : inline;
    }

    if (s.node == e.node) {
      return nodeText(s.node, s.offset, e.offset);
    }
    // Blank line between blocks so headings/lists parse as Markdown — but
    // consecutive quote blocks of one group join with a SINGLE newline: a
    // blank line is the Markdown boundary between blockquotes, so the
    // round-trip would mark every line `qbreak` and the quote bar shatters.
    final buf = StringBuffer(
        nodeText(s.node, s.offset, nodes[s.node].text.length));
    for (var i = s.node + 1; i <= e.node; i++) {
      final sameQuoteGroup = nodes[i].kind == 'quote' &&
          nodes[i - 1].kind == 'quote' &&
          nodes[i].data['qbreak'] != true;
      buf.write(sameQuoteGroup ? '\n' : '\n\n');
      buf.write(i == e.node
          ? nodeText(e.node, 0, e.offset)
          : nodeText(i, 0, nodes[i].text.length));
    }
    return buf.toString();
  }

  /// The leading Markdown marker for a block kind (heading/list/quote/todo).
  /// A non-quote block that lives INSIDE a blockquote (`data.quote` > 0 on a
  /// heading/list/code) keeps its `> ` markers — dropping them stripped the
  /// quote from every such block on copy.
  String _blockPrefix(EditorNode node) {
    final pad = node.isListKind ? '    ' * node.indent : '';
    final quote = node.kind == 'quote' ? '' : '> ' * node.quoteDepth.clamp(0, 16);
    switch (node.kind) {
      case 'heading':
        return '$quote${'#' * node.headingLevel} ';
      case 'bulleted_list':
        // `data.marker` records a marker change that separates two adjacent
        // lists — serialize it or the lists merge back into one on paste.
        return '$quote$pad${(node.data['marker'] as String?) ?? '-'} ';
      case 'numbered_list':
        // The real start number: `5. five` must not paste back as `1.`.
        return '$quote$pad${node.data['start'] ?? 1}. ';
      case 'quote':
        // Nested quotes repeat the marker, matching the Rust exporter.
        return '> ' * node.quoteDepth.clamp(1, 16);
      case 'todo':
        return node.todoChecked ? '$quote$pad- [x] ' : '$quote$pad- [ ] ';
      case 'footnote_def':
        // GFM definition leader; continuation lines indent 4 columns, but the
        // copy path is line-oriented so the prefix covers the first line.
        return '$quote[^${node.data['label'] ?? ''}]: ';
      default:
        return quote;
    }
  }

  /// The (node, from, to) slices the current ranged selection covers, in order.
  /// Empty when there is no ranged selection. Shared by the plain/HTML copy
  /// flavors ([selectionText] keeps its own walk for the quote-group newline).
  List<({int node, int from, int to})> _selectionSlices() {
    final sel = selection;
    if (sel == null || sel.isCollapsed) return const [];
    final s = sel.start, e = sel.end;
    if (s.node == e.node) {
      return [(node: s.node, from: s.offset, to: e.offset)];
    }
    final out = <({int node, int from, int to})>[
      (node: s.node, from: s.offset, to: nodes[s.node].text.length),
    ];
    for (var i = s.node + 1; i <= e.node; i++) {
      out.add((
        node: i,
        from: 0,
        to: i == e.node ? e.offset : nodes[i].text.length,
      ));
    }
    return out;
  }

  /// The selection as human-readable PLAIN text — what you see, with no Markdown
  /// syntax (so pasting into Notepad/etc. doesn't leak `**`/`#`/`` ` ``). Inline
  /// marks live outside `text`, so a raw slice is already unmarked; only the
  /// rendered block affordance (bullet / number / checkbox) is added back. The
  /// clipboard's `text/plain` flavor — the `text/html` one keeps formatting.
  String selectionPlainText({Map<String, String>? imageUrls}) {
    final slices = _selectionSlices();
    if (slices.isEmpty) return '';
    final buf = StringBuffer();
    final counters = <int, int>{}; // numbered-list running count per indent
    for (var i = 0; i < slices.length; i++) {
      final sl = slices[i];
      final node = nodes[sl.node];
      if (i > 0) {
        final prev = nodes[slices[i - 1].node];
        final sameQuoteGroup = node.kind == 'quote' &&
            prev.kind == 'quote' &&
            node.data['qbreak'] != true;
        buf.write(sameQuoteGroup ? '\n' : '\n\n');
      }
      buf.write(_nodePlain(node, sl.from, sl.to, imageUrls, counters));
    }
    return buf.toString();
  }

  String _nodePlain(EditorNode node, int from, int to,
      Map<String, String>? imageUrls, Map<int, int> counters) {
    // Numbering mirrors the renderer (render.dart): a non-numbered LIST item
    // interrupts only its own level and deeper (parents keep counting); any
    // other block ends every run. Wholesale clear() on a nested bullet made
    // the copied numbers contradict the numbers on screen.
    if (node.kind != 'numbered_list') {
      if (node.isListKind) {
        counters.removeWhere((k, _) => k >= node.indent);
      } else {
        counters.clear();
      }
    }
    switch (node.kind) {
      case 'table':
        return TableData.fromBlock(node.data)
            .rows
            .map((r) => r.join('\t'))
            .join('\n');
      case 'image':
        final fileId = node.data['file_id'] as String?;
        return (imageUrls?[fileId] ?? node.data['url'] ?? node.text) as String;
      case 'divider':
        return '';
    }
    final len = node.text.length;
    final a = from.clamp(0, len);
    final b = to.clamp(0, len);
    final sub = node.text.substring(a, b);
    if (!(a == 0 && b == len)) return sub; // partial line → bare text
    final indent = node.isListKind ? '  ' * node.indent : '';
    switch (node.kind) {
      case 'bulleted_list':
        return '$indent• $sub';
      case 'numbered_list':
        // Returning to a shallower level restarts the deeper runs, and the
        // first item of a run seeds from its stored start (`5.` stays `5.`).
        counters.removeWhere((k, _) => k > node.indent);
        final n = (counters[node.indent] ??
                ((node.data['start'] as int?) ?? 1) - 1) +
            1;
        counters[node.indent] = n;
        return '$indent$n. $sub';
      case 'todo':
        return '$indent${node.todoChecked ? '☑' : '☐'} $sub';
      default:
        return sub; // heading / quote / paragraph → just the text
    }
  }

  /// A run of consecutive list items → nested `<ul>`/`<ol>` HTML. An item
  /// whose `data.indent` is deeper than the current level opens a sublist
  /// INSIDE the previous `<li>` (`</li>` is written lazily so the nested list
  /// sits within it — valid HTML, what Typora/Word expect); a shallower item
  /// pops back out. A kind change at the same level (`ul` ↔ `ol`) closes the
  /// current list and opens the other.
  static String _listRunHtml(
    List<EditorNode> items,
    String Function(EditorNode) content,
  ) {
    var i = 0;
    String emit(int level) {
      final sb = StringBuffer();
      String? tag;
      var liOpen = false;
      while (i < items.length) {
        final it = items[i];
        final lv = it.indent < 0 ? 0 : it.indent;
        if (lv < level) break;
        if (lv > level) {
          // Deeper item: nest inside the open <li>; a run that STARTS deep
          // (selection began on a nested item) just emits the sublist bare.
          sb.write(emit(level + 1));
          continue;
        }
        final want = it.kind == 'numbered_list' ? 'ol' : 'ul';
        if (tag != want) {
          if (liOpen) {
            sb.write('</li>');
            liOpen = false;
          }
          if (tag != null) sb.write('</$tag>');
          // `<ol start="5">`: the run's real first number must survive the
          // trip — pasting `5. five` back as `1.` changes visible content.
          final start = want == 'ol' ? it.data['start'] : null;
          sb.write(start is int && start != 1 ? '<ol start="$start">' : '<$want>');
          tag = want;
        }
        if (liOpen) sb.write('</li>');
        sb.write('<li>${content(it)}');
        liOpen = true;
        i++;
      }
      if (liOpen) sb.write('</li>');
      if (tag != null) sb.write('</$tag>');
      return sb.toString();
    }

    return emit(0);
  }

  /// The selection as HTML — the clipboard's rich flavor, built straight from the
  /// block model (headings, lists, quotes, code, tables + inline marks) so
  /// Markdown editors that read `text/html` keep the formatting. Consecutive list
  /// items / quotes group into one `<ul>`/`<ol>`/`<blockquote>`; nested list
  /// levels (`data.indent`) become nested `<ul>`/`<ol>`.
  String selectionHtml({Map<String, String>? imageUrls}) {
    final slices = _selectionSlices();
    if (slices.isEmpty) return '';
    final buf = StringBuffer();
    // Open <blockquote> nesting: adjusted per block to its quoteDepth, so
    // depth-2 quotes emit nested blockquotes and heading/list/code blocks
    // living INSIDE a quote (data.quote > 0) keep their wrapper instead of
    // being dumped outside it.
    var quoteDepth = 0;
    void setQuote(int depth) {
      while (quoteDepth > depth) {
        buf.write('</blockquote>');
        quoteDepth--;
      }
      while (quoteDepth < depth) {
        buf.write('<blockquote>');
        quoteDepth++;
      }
    }

    String inlineHtmlOf(EditorNode node, int a, int b) {
      final clipped = <Mark>[];
      for (final m in marksFromData(node.data)) {
        final s = m.start.clamp(a, b);
        final e = m.end.clamp(a, b);
        if (e > s) {
          clipped.add(Mark(s - a, e - a, m.type, href: m.href, title: m.title));
        }
      }
      return inlineToHtml(node.text.substring(a, b), clipped);
    }

    bool fullListItem(({int node, int from, int to}) sl) {
      final node = nodes[sl.node];
      final len = node.text.length;
      if (!(sl.from.clamp(0, len) == 0 && sl.to.clamp(0, len) == len)) {
        return false;
      }
      return node.kind == 'bulleted_list' ||
          node.kind == 'numbered_list' ||
          node.kind == 'todo';
    }

    var si = 0;
    while (si < slices.length) {
      final sl = slices[si];
      final node = nodes[sl.node];
      final len = node.text.length;
      final a = sl.from.clamp(0, len);
      final b = sl.to.clamp(0, len);
      final full = a == 0 && b == len;

      if (node.kind == 'image') {
        setQuote(0);
        final fileId = node.data['file_id'] as String?;
        final src = (imageUrls?[fileId] ??
            node.data['url'] ??
            node.data['name'] ??
            '') as String;
        buf.write('<p><img src="${escapeHtmlAttr(src)}" '
            'alt="${escapeHtmlAttr(node.text)}"></p>');
        si++;
        continue;
      }
      if (node.kind == 'divider') {
        setQuote(0);
        buf.write('<hr>');
        si++;
        continue;
      }
      if (node.kind == 'table') {
        setQuote(0);
        buf.write(_tableHtml(TableData.fromBlock(node.data)));
        si++;
        continue;
      }
      if (node.kind == 'code_block') {
        setQuote(full ? node.quoteDepth : 0);
        final lang = _copyLanguage(node);
        final cls = lang.isEmpty
            ? ''
            : ' class="language-${escapeHtmlAttr(lang)}"';
        buf.write('<pre><code$cls>${escapeHtml(node.text.substring(a, b))}'
            '</code></pre>');
        si++;
        continue;
      }
      if (node.kind == 'math_block') {
        // `$$…$$` inside a data-mica-math wrapper: external editors see the
        // TeX; our converter passes it through verbatim so it re-parses as a
        // math block instead of a paragraph of raw LaTeX.
        setQuote(0);
        buf.write('<p><span data-mica-math="1">\$\$'
            '${escapeHtml(node.text)}\$\$</span></p>');
        si++;
        continue;
      }
      if (node.kind == 'footnote_def') {
        // The GFM definition leader travels in an attribute; the converter
        // reconstructs `[^label]: …` (escaping would kill a literal leader).
        setQuote(0);
        final label = (node.data['label'] ?? '') as String;
        buf.write('<p data-mica-fndef="${escapeHtmlAttr(label)}">'
            '${inlineHtmlOf(node, a, b)}</p>');
        si++;
        continue;
      }

      if (fullListItem(sl)) {
        // Collect the unbroken run of fully-selected list items AT THE SAME
        // quote depth, then emit it as PROPERLY NESTED <ul>/<ol> from each
        // item's `data.indent` — the old flat single-<ul> writer erased
        // nesting, so pasting a multi-level list (even mica → mica, where the
        // HTML flavor wins) lost its levels.
        setQuote(node.quoteDepth);
        final run = <EditorNode>[];
        while (si < slices.length &&
            fullListItem(slices[si]) &&
            nodes[slices[si].node].quoteDepth == node.quoteDepth) {
          run.add(nodes[slices[si].node]);
          si++;
        }
        buf.write(_listRunHtml(run, (n) {
          final box = n.kind == 'todo'
              ? '<input type="checkbox"${n.todoChecked ? ' checked' : ''}'
                  ' disabled> '
              : '';
          return '$box${inlineHtmlOf(n, 0, n.text.length)}';
        }));
        continue;
      }

      if (full && node.kind == 'quote') {
        // qbreak marks the start of a NEW quote group: close the open one so
        // two separate bars don't fuse into a single blockquote on paste.
        if (node.data['qbreak'] == true) setQuote(0);
        setQuote(node.quoteDepth.clamp(1, 16));
        buf.write('<p>${inlineHtmlOf(node, a, b)}</p>');
        si++;
        continue;
      }

      if (full && node.kind == 'heading') {
        setQuote(node.quoteDepth);
        final lvl = node.headingLevel.clamp(1, 6);
        buf.write('<h$lvl>${inlineHtmlOf(node, a, b)}</h$lvl>');
        si++;
        continue;
      }
      setQuote(full ? node.quoteDepth : 0);
      buf.write('<p>${inlineHtmlOf(node, a, b)}</p>');
      si++;
    }
    setQuote(0);
    return buf.toString();
  }

  String _tableHtml(TableData table) {
    final buf = StringBuffer('<table>');
    for (var r = 0; r < table.rows.length; r++) {
      buf.write('<tr>');
      final tag = r == 0 ? 'th' : 'td';
      for (final cell in table.rows[r]) {
        buf.write('<$tag>${escapeHtml(cell)}</$tag>');
      }
      buf.write('</tr>');
    }
    buf.write('</table>');
    return buf.toString();
  }

  /// Delete the current ranged selection, which may span multiple nodes. The
  /// surrounding text of the first and last nodes is merged into the first node
  /// and the nodes in between are removed. The document never becomes empty.
  /// Returns false when the selection is absent or collapsed.
  bool deleteSelection() {
    final sel = selection;
    if (sel == null || sel.isCollapsed) return false;
    final start = sel.start;
    final end = sel.end;

    if (start.node == end.node) {
      final node = nodes[start.node];
      final s = start.offset.clamp(0, node.text.length);
      final e = end.offset.clamp(0, node.text.length);
      node.text = node.text.substring(0, s) + node.text.substring(e);
      // Marks die with their text — without this the ranges stay put and
      // bleed onto whatever slides left into them (e.g. a deleted link's
      // format infecting the following words).
      final marks = shiftMarks(
        marksFromData(node.data),
        s,
        e,
        s - e,
        node.text.length,
      );
      node.data = {...node.data, 'marks': marksToJson(marks)};
      _dirty.remove(node.id);
      _sendNow([
        {
          'type': 'update_block',
          'block_id': node.id,
          'text': node.text,
          'data': node.data,
        },
      ]);
      collapseTo(DocPosition(start.node, s));
      return true;
    }

    final startNode = nodes[start.node];
    final endNode = nodes[end.node];
    // Unit blocks (atomic kinds, rendered diagrams) are consumed whole by a
    // selection that touches them: their text must never merge into a
    // neighbor (a diagram's source spilling into a paragraph reads as stray
    // code), and a unit start block is reborn as the plain paragraph that
    // carries whatever survives the cut.
    final startIsUnit = startNode.isUnitBlock;
    final endIsUnit = endNode.isUnitBlock;
    final s = startIsUnit ? 0 : start.offset.clamp(0, startNode.text.length);
    final e = end.offset.clamp(0, endNode.text.length);
    final prefix = startIsUnit ? '' : startNode.text.substring(0, s);
    final suffix = endIsUnit ? '' : endNode.text.substring(e);
    final merged = prefix + suffix;
    final removed = [for (var i = start.node + 1; i <= end.node; i++) nodes[i].id];

    // Start node keeps marks strictly before the cut (clip, don't stretch —
    // the joined tail must not inherit them); the end node's surviving tail
    // brings its own marks along, remapped to the junction.
    final (startMarks, _) = startIsUnit
        ? (const <Mark>[], const <Mark>[])
        : splitMarks(marksFromData(startNode.data), s);
    final tailMarks = <Mark>[];
    if (!endIsUnit) {
      for (final m in marksFromData(endNode.data)) {
        final ms = (m.start - e + s).clamp(s, merged.length);
        final me = (m.end - e + s).clamp(s, merged.length);
        if (me > ms) tailMarks.add(Mark(ms, me, m.type, href: m.href));
      }
    }

    if (startIsUnit) {
      startNode
        ..kind = 'paragraph'
        ..data = {};
    }
    startNode.text = merged;
    startNode.data = {
      ...startNode.data,
      'marks': marksToJson([...startMarks, ...tailMarks]),
    };
    nodes.removeRange(start.node + 1, end.node + 1);
    _dirty.remove(startNode.id);
    final ops = <DocOp>[
      {
        'type': 'update_block',
        'block_id': startNode.id,
        if (startIsUnit) 'kind': 'paragraph',
        'text': merged,
        'data': startNode.data,
      },
    ];
    for (final id in removed) {
      _dirty.remove(id);
      ops.add({'type': 'delete_block', 'block_id': id});
    }
    _sendNow(ops);
    collapseTo(DocPosition(start.node, s));
    return true;
  }

  /// Change the focused node's kind (turn-into / slash menu).
  void setFocusedKind(String kind, [Map<String, dynamic>? data]) {
    final node = focusedNode;
    if (node == null) return;
    node.kind = kind;
    node.data = data ?? {};
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'kind': kind, 'data': node.data},
    ]);
    notifyListeners();
  }

  /// Convert every (non-atomic) block in the current selection to [kind] — for
  /// turning a multi-line selection into a list / quote / heading / todo, or
  /// merging the lines into one code block. Inline marks are preserved.
  void setSelectedBlocksKind(String kind, {Map<String, dynamic>? data}) {
    final sel = selection;
    if (sel == null || nodes.isEmpty) return;
    final lo = sel.start.node.clamp(0, nodes.length - 1);
    final hi = sel.end.node.clamp(0, nodes.length - 1);

    if (kind == 'code_block') {
      // Merge the selected blocks' text into a single code block.
      final texts = <String>[];
      String? firstId;
      final removeIds = <String>[];
      for (var i = lo; i <= hi; i++) {
        final node = nodes[i];
        if (node.isAtomic) continue;
        if (firstId == null) {
          firstId = node.id;
        } else {
          removeIds.add(node.id);
        }
        texts.add(node.text);
      }
      if (firstId == null) return;
      final firstIndex = nodes.indexWhere((n) => n.id == firstId);
      final merged = texts.join('\n');
      final ops = <DocOp>[];
      nodes[firstIndex]
        ..kind = 'code_block'
        ..text = merged
        ..data = {};
      _dirty.remove(firstId);
      ops.add({
        'type': 'update_block',
        'block_id': firstId,
        'kind': 'code_block',
        'text': merged,
        'data': <String, dynamic>{},
      });
      nodes.removeWhere((n) => removeIds.contains(n.id));
      for (final id in removeIds) {
        _dirty.remove(id);
        ops.add({'type': 'delete_block', 'block_id': id});
      }
      _sendNow(ops);
      collapseTo(DocPosition(firstIndex, merged.length));
      return;
    }

    final ops = <DocOp>[];
    for (var i = lo; i <= hi; i++) {
      final node = nodes[i];
      if (node.isAtomic || node.kind == 'code_block') continue;
      final marks = node.data['marks'];
      final nd = <String, dynamic>{
        if (marks != null) 'marks': marks,
        ...?data,
      };
      node
        ..kind = kind
        ..data = nd;
      ops.add({
        'type': 'update_block',
        'block_id': node.id,
        'kind': kind,
        'data': nd,
      });
    }
    if (ops.isNotEmpty) {
      _sendNow(ops);
      notifyListeners();
    }
  }

  /// The language label to put on a code block **leaving Mica** — the
  /// clipboard, in Markdown or HTML.
  ///
  /// An `auto` block stores no language at all (detection runs live, at paste
  /// time), so copying one emitted a bare ``` and Typora/GitHub/VS Code got a
  /// block with no language and no way to work one out: on their side `auto`
  /// simply doesn't exist. So the clipboard drops the idea of `auto` and hands
  /// over the resolved answer.
  ///
  /// Deliberately NOT what `export_markdown` does (crates/markdown). Export is
  /// document serialization and round-trip is an invariant there (CLAUDE.md #4;
  /// `conformance.rs::fixtures_round_trip` pins it, bare fences and all) — the
  /// author wrote no language, so the file gets no language. The clipboard has
  /// no such contract: it is interchange, aimed at a reader that cannot detect.
  ///
  /// A label the author pinned goes out **verbatim** — `py` stays `py`. It is
  /// their word, it is not wrong, and canonicalising it would be churn (the
  /// same reason `retagMislabeledFences` leaves a correct alias alone). Only an
  /// auto block, which has no word of its own, gets detection filled in — and
  /// `plaintext` still goes out bare, because that is detection saying "no
  /// idea" and stamping the guess on would dress it up as a decision.
  static String _copyLanguage(EditorNode node) {
    final pinned = (node.data['language'] as String?)?.trim() ?? '';
    if (pinned.isNotEmpty && pinned != 'auto') return pinned;
    final detected = detectLanguage(node.text);
    return detected == 'plaintext' ? '' : detected;
  }

  /// Set a code block's language (null/`auto` clears it, re-enabling detection).
  void setCodeLanguage(int index, String? language) {
    if (index < 0 || index >= nodes.length) return;
    final node = nodes[index];
    if (node.kind != 'code_block') return;
    final data = {...node.data};
    if (language == null || language == 'auto') {
      data.remove('language');
    } else {
      data['language'] = language;
    }
    node.data = data;
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  // ---------------------------------------------------------------------------
  // Tables
  // ---------------------------------------------------------------------------

  void setTableCell(int index, int row, int col, String text) {
    if (index < 0 || index >= nodes.length) return;
    final node = nodes[index];
    if (node.kind != 'table') return;
    final table = TableData.fromBlock(node.data);
    if (row < 0 || row >= table.rows.length) return;
    if (col < 0 || col >= table.rows[row].length) return;
    table.rows[row][col] = text;
    node.data = table.toBlockData();
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  /// Update a cell's text LOCALLY — relayout only, NO op sent. Used while a cell
  /// is being edited so its row grows/shrinks live as you type (each newline
  /// re-measures the cell and re-lays out the table). The value is persisted by
  /// [setTableCell] on commit. A no-op when the text is unchanged, so a mere
  /// selection change doesn't trigger a relayout.
  void previewTableCell(int index, int row, int col, String text) {
    if (index < 0 || index >= nodes.length) return;
    final node = nodes[index];
    if (node.kind != 'table') return;
    final table = TableData.fromBlock(node.data);
    if (row < 0 || row >= table.rows.length) return;
    if (col < 0 || col >= table.rows[row].length) return;
    if (table.rows[row][col] == text) return;
    table.rows[row][col] = text;
    node.data = table.toBlockData();
    notifyListeners(); // relayout only; not persisted until commit
  }

  void _writeTable(int index, TableData table) {
    final node = nodes[index];
    node.data = table.toBlockData();
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  TableData? _tableAt(int index) {
    if (index < 0 || index >= nodes.length) return null;
    if (nodes[index].kind != 'table') return null;
    return TableData.fromBlock(nodes[index].data);
  }

  /// Blank every cell in the inclusive [r0],[c0]–[r1],[c1] rectangle (one op) —
  /// Delete/Backspace/cut on a cell-area selection.
  void clearTableCells(int index, int r0, int c0, int r1, int c1) {
    final table = _tableAt(index);
    if (table == null) return;
    var changed = false;
    for (var r = r0.clamp(0, table.rows.length); r <= r1 && r < table.rows.length; r++) {
      for (var c = c0.clamp(0, table.rows[r].length);
          c <= c1 && c < table.rows[r].length;
          c++) {
        if (table.rows[r][c].isNotEmpty) {
          table.rows[r][c] = '';
          changed = true;
        }
      }
    }
    if (changed) _writeTable(index, table);
  }

  /// Reset every column to the same weight — the renderer reads "all equal" as
  /// auto-fit-to-content mode, so this re-enables automatic column widths after
  /// manual resizes.
  void resetTableColumnWidths(int index) {
    final table = _tableAt(index);
    if (table == null) return;
    for (var c = 0; c < table.widths.length; c++) {
      table.widths[c] = 1.0;
    }
    _writeTable(index, table);
  }

  void insertTableRow(int index, int at) {
    final table = _tableAt(index);
    if (table == null) return;
    table.insertRow(at);
    _writeTable(index, table);
  }

  void deleteTableRow(int index, int at) {
    final table = _tableAt(index);
    if (table == null) return;
    table.deleteRow(at);
    _writeTable(index, table);
  }

  void insertTableColumn(int index, int at) {
    final table = _tableAt(index);
    if (table == null) return;
    table.insertColumn(at);
    _writeTable(index, table);
  }

  void deleteTableColumn(int index, int at) {
    final table = _tableAt(index);
    if (table == null) return;
    table.deleteColumn(at);
    _writeTable(index, table);
  }

  /// Reorder a column by [delta] positions (the column menu's move
  /// left/right). Cell contents and the column width travel together.
  void moveTableColumn(int index, int col, int delta) {
    final table = _tableAt(index);
    if (table == null) return;
    final to = col + delta;
    if (col < 0 || to < 0 || col >= table.columns || to >= table.columns) {
      return;
    }
    for (final row in table.rows) {
      if (col >= row.length || to >= row.length) continue;
      final v = row.removeAt(col);
      row.insert(to, v);
    }
    if (col < table.widths.length && to < table.widths.length) {
      final w = table.widths.removeAt(col);
      table.widths.insert(to, w);
    }
    _writeTable(index, table);
  }

  /// Reorder a body row by [delta] positions (the header row stays put).
  void moveTableRow(int index, int row, int delta) {
    final table = _tableAt(index);
    if (table == null) return;
    final first = table.header ? 1 : 0;
    final to = row + delta;
    if (row < first || to < first || row >= table.rows.length || to >= table.rows.length) {
      return;
    }
    final r = table.rows.removeAt(row);
    table.rows.insert(to, r);
    _writeTable(index, table);
  }

  void setTableAlign(int index, String align) {
    final table = _tableAt(index);
    if (table == null) return;
    _writeTable(
      index,
      TableData(
        table.rows,
        header: table.header,
        align: align,
        tableWidth: table.tableWidth,
        widths: table.widths,
      ),
    );
  }

  /// Live preview of column widths during a drag (no op sent / no save).
  /// Live preview of the overall table width while dragging its right edge.
  void previewTableWidth(int index, double fraction) {
    final table = _tableAt(index);
    if (table == null) return;
    nodes[index].data = TableData(
      table.rows,
      header: table.header,
      align: table.align,
      tableWidth: fraction.clamp(0.15, 1.0),
      widths: table.widths,
    ).toBlockData();
    notifyListeners();
  }

  /// Persist the overall table width (drag end).
  void setTableWidth(int index, double fraction) {
    final table = _tableAt(index);
    if (table == null) return;
    _writeTable(
      index,
      TableData(
        table.rows,
        header: table.header,
        align: table.align,
        tableWidth: fraction.clamp(0.15, 1.0),
        widths: table.widths,
      ),
    );
  }

  void previewTableColumnWidths(int index, List<double> widths) {
    final table = _tableAt(index);
    if (table == null) return;
    for (var i = 0; i < table.widths.length && i < widths.length; i++) {
      table.widths[i] = widths[i] <= 0 ? 0.05 : widths[i];
    }
    nodes[index].data = table.toBlockData();
    notifyListeners();
  }

  void setTableColumnWidths(int index, List<double> widths) {
    final table = _tableAt(index);
    if (table == null) return;
    for (var i = 0; i < table.widths.length && i < widths.length; i++) {
      table.widths[i] = widths[i] <= 0 ? 0.05 : widths[i];
    }
    _writeTable(index, table);
  }

  /// Remove a node entirely (used to delete a table). Keeps the document
  /// non-empty.
  void deleteNode(int index) {
    if (index < 0 || index >= nodes.length) return;
    final removed = nodes[index].id;
    nodes.removeAt(index);
    _sendNow([
      {'type': 'delete_block', 'block_id': removed},
    ]);
    if (nodes.isEmpty) {
      ensureNotEmpty();
      return;
    }
    final i = index.clamp(0, nodes.length - 1);
    collapseTo(DocPosition(i, nodes[i].text.length));
  }

  /// Toggle line wrapping for a code block.
  void toggleCodeWrap(int index) {
    if (index < 0 || index >= nodes.length) return;
    final node = nodes[index];
    if (node.kind != 'code_block') return;
    final data = {...node.data};
    if (node.data['wrap'] == true) {
      data.remove('wrap');
    } else {
      data['wrap'] = true;
    }
    node.data = data;
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  /// Switch a diagram code block (```mermaid) between its rendered preview
  /// and source editing. Stored as data.view ('code'); absent = preview, the
  /// default — readers want the picture, not the source.
  void setCodeView(int index, String view) {
    if (index < 0 || index >= nodes.length) return;
    final node = nodes[index];
    if (node.kind != 'code_block') return;
    final data = {...node.data};
    if (view == 'code') {
      data['view'] = 'code';
    } else {
      data.remove('view');
    }
    node.data = data;
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  void toggleTodo(int index) {
    if (index < 0 || index >= nodes.length) return;
    final node = nodes[index];
    if (node.kind != 'todo') return;
    final checked = !(node.data['checked'] == true);
    node.data = {...node.data, 'checked': checked};
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  /// Markdown input rules: when the caret sits right after a line-start marker
  /// the user just typed, convert the block and strip the marker. Returns true
  /// if a conversion happened. Chained: `- ` → bullet, then `[ ] ` → todo.
  bool applyInputRules() {
    final sel = selection;
    if (sel == null || !sel.isCollapsed) return false;
    final i = sel.focus.node;
    if (i >= nodes.length) return false;
    final node = nodes[i];
    final text = node.text;
    final caret = sel.focus.offset;

    bool convert(String kind, Map<String, dynamic> data, int stripLen) {
      final rest = text.substring(stripLen);
      node
        ..kind = kind
        ..data = data
        ..text = rest;
      _dirty.remove(node.id);
      _sendNow([
        {
          'type': 'update_block',
          'block_id': node.id,
          'kind': kind,
          'data': data,
          'text': rest,
        },
      ]);
      collapseTo(DocPosition(i, 0));
      return true;
    }

    // `# ` works on paragraphs AND existing headings — typing the marker on
    // a title re-levels it (Typora), instead of doing nothing.
    if (node.kind == 'paragraph' || node.kind == 'heading') {
      for (var lvl = 6; lvl >= 1; lvl--) {
        final marker = '${'#' * lvl} ';
        if (caret == marker.length && text.startsWith(marker)) {
          return convert('heading', {'level': lvl}, marker.length);
        }
      }
    }

    if (node.kind == 'paragraph') {
      if (caret == 2 &&
          (text.startsWith('- ') || text.startsWith('* ') || text.startsWith('+ '))) {
        return convert('bulleted_list', {}, 2);
      }
      if (caret == 2 && text.startsWith('> ')) {
        return convert('quote', {}, 2);
      }
      final numbered = RegExp(r'^\d+\. ').firstMatch(text);
      if (numbered != null && caret == numbered.end) {
        return convert('numbered_list', {}, numbered.end);
      }
      if (caret == 3 && (text == '```' || text == '~~~')) {
        return convert('code_block', {}, 3);
      }
      // Typing dollar-dollar…dollar-dollar converts to a math block on close.
      if (caret == text.length &&
          text.length > 4 &&
          text.startsWith(r'$$') &&
          text.endsWith(r'$$')) {
        final src = text.substring(2, text.length - 2).trim();
        if (src.isNotEmpty && !src.contains(r'$$')) {
          node
            ..kind = 'math_block'
            ..text = src
            ..data = {};
          _dirty.remove(node.id);
          final ops = <DocOp>[
            {
              'type': 'update_block',
              'block_id': node.id,
              'kind': 'math_block',
              'text': src,
              'data': <String, dynamic>{},
            },
          ];
          // Atomic block: park the caret on a paragraph after it.
          final after = i + 1;
          if (after >= nodes.length || nodes[after].isAtomic) {
            final p = EditorNode(id: _genId(), kind: 'paragraph', text: '');
            nodes.insert(after, p);
            ops.add(_insertOp(p, after));
          }
          _sendNow(ops);
          collapseTo(DocPosition(after, 0));
          return true;
        }
      }
      if (caret == 3 && (text == '---' || text == '***' || text == '___')) {
        node.text = '';
        insertDivider();
        return true;
      }
    } else if (node.kind == 'bulleted_list') {
      for (final t in const ['[ ] ', '[] ']) {
        if (caret == t.length && text.startsWith(t)) {
          return convert('todo', {'checked': false}, t.length);
        }
      }
      for (final t in const ['[x] ', '[X] ']) {
        if (caret == t.length && text.startsWith(t)) {
          return convert('todo', {'checked': true}, t.length);
        }
      }
    }

    // Inline rules: a just-typed closing marker (or `)` for a link) converts the
    // wrapped run into a mark and strips the syntax. Code blocks carry no marks.
    if (node.kind != 'code_block' && node.kind != 'table') {
      if (_applyInlineRule(node, i, text, caret)) return true;
    }
    return false;
  }

  /// Detect an inline-Markdown span ending at [caret] and, if found, strip its
  /// markers, apply the mark (shifting existing marks), and place the caret after
  /// the converted run. Returns true on conversion.
  bool _applyInlineRule(EditorNode node, int i, String text, int caret) {
    // Resolved span to convert: text[innerStart, innerEnd) becomes the marked
    // run after the markers in [matchStart, caret) are removed.
    int matchStart = -1, innerStart = -1, innerEnd = -1;
    String? type;
    String? href;

    // Link: [label](url) — closing `)` just typed.
    if (text.endsWith(')')) {
      final m = RegExp(r'\[([^\]]+)\]\(([^)\s]+)\)$')
          .firstMatch(text.substring(0, caret));
      if (m != null) {
        matchStart = m.start;
        innerStart = m.start + 1; // after '['
        innerEnd = m.start + 1 + m.group(1)!.length; // before ']'
        type = 'link';
        href = m.group(2);
      }
    }

    // Two-char wrappers: **bold**, ~~strike~~.
    if (type == null && caret >= 4) {
      for (final mk in const [('**', 'bold'), ('~~', 'strike')]) {
        if (text.substring(caret - 2, caret) != mk.$1) continue;
        final open = text.lastIndexOf(mk.$1, caret - 3);
        if (open < 0) continue;
        final s = open + 2;
        final e = caret - 2;
        if (e <= s || text.substring(s, e).trim().isEmpty) continue;
        matchStart = open;
        innerStart = s;
        innerEnd = e;
        type = mk.$2;
        break;
      }
    }

    // One-char wrappers: `code`, *italic*, _italic_.
    if (type == null && caret >= 3) {
      for (final mk in const [
        ('`', 'code'),
        ('*', 'italic'),
        ('_', 'italic'),
        ('~', 'strike'),
      ]) {
        final ch = mk.$1;
        if (text[caret - 1] != ch) continue;
        final open = text.lastIndexOf(ch, caret - 2);
        if (open < 0) continue;
        // Skip markers that are part of a two-char wrapper (e.g. the `*` in `**`).
        if (open > 0 && text[open - 1] == ch) continue;
        if (text[caret - 2] == ch) continue;
        final s = open + 1;
        final e = caret - 1;
        if (e <= s || text.substring(s, e).trim().isEmpty) continue;
        matchStart = open;
        innerStart = s;
        innerEnd = e;
        type = mk.$2;
        break;
      }
    }

    if (type == null) return false;

    final inner = text.substring(innerStart, innerEnd);
    final newText = text.substring(0, matchStart) + inner + text.substring(caret);

    // Shift existing marks for the two marker deletions (right side first so the
    // left offsets stay valid), then add the new mark over the converted run.
    var marks = marksFromData(node.data);
    // Right deletion: [innerEnd, caret).
    marks = shiftMarks(
      marks,
      innerEnd,
      caret,
      innerEnd - caret,
      text.length - (caret - innerEnd),
    );
    // Left deletion: [matchStart, innerStart).
    marks = shiftMarks(
      marks,
      matchStart,
      innerStart,
      matchStart - innerStart,
      newText.length,
    );
    final markEnd = matchStart + inner.length;
    marks = applyMark(marks, matchStart, markEnd, type, href: href, add: true);

    node
      ..text = newText
      ..data = {...node.data, 'marks': marksToJson(marks)};
    _dirty.remove(node.id);
    _sendNow([
      {
        'type': 'update_block',
        'block_id': node.id,
        'text': newText,
        'data': node.data,
      },
    ]);
    collapseTo(DocPosition(i, markEnd));
    return true;
  }

  /// Apply a slash-menu choice: remove the `/query` text in [start, end) and
  /// convert the focused block to [kind].
  void applySlashCommand(int start, int end, String kind, Map<String, dynamic> data) {
    final sel = selection;
    if (sel == null) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;
    final node = nodes[i];
    final s = start.clamp(0, node.text.length);
    final e = end.clamp(s, node.text.length);
    final newText = node.text.substring(0, s) + node.text.substring(e);
    node
      ..kind = kind
      ..data = Map<String, dynamic>.from(data)
      ..text = newText;
    _dirty.remove(node.id);
    final ops = <DocOp>[
      {
        'type': 'update_block',
        'block_id': node.id,
        'kind': kind,
        'data': node.data,
        'text': newText,
      },
    ];
    if (node.isAtomic) {
      // Atomic block: the caret can't live inside it (an IME echo would
      // clobber the block). Park it on a paragraph after, as the `$$…$$`
      // input rule does.
      final after = i + 1;
      if (after >= nodes.length || nodes[after].isAtomic) {
        final p = EditorNode(id: _genId(), kind: 'paragraph', text: '');
        nodes.insert(after, p);
        ops.add(_insertOp(p, after));
      }
      _sendNow(ops);
      collapseTo(DocPosition(after, 0));
      return;
    }
    _sendNow(ops);
    collapseTo(DocPosition(i, s));
  }

  /// Replace the link over `[start, end)` of node [i]: with an [href] the
  /// range gets that link; with null the link is removed (text untouched).
  /// Used by the link hover toolbar (edit / remove).
  void setLinkRange(int i, int start, int end, String? href) {
    if (i < 0 || i >= nodes.length) return;
    final node = nodes[i];
    final marks = marksFromData(node.data);
    var next = applyMark(marks, start, end, 'link', add: false);
    if (href != null && href.isNotEmpty) {
      next = applyMark(next, start, end, 'link', href: href, add: true);
    }
    node.data = {...node.data, 'marks': marksToJson(next)};
    _dirty.remove(node.id);
    _sendNow([
      {
        'type': 'update_block',
        'block_id': node.id,
        'text': node.text,
        'data': node.data,
      },
    ]);
    notifyListeners();
  }

  /// Replace `[from, to)` in the focused node with [title] carrying a link
  /// mark to [href] — used by the `[[` page-link picker.
  void insertPageLink(int from, int to, String title, String href) {
    final sel = selection;
    if (sel == null) return;
    replaceLink(sel.focus.node, from, to, title, href);
  }

  /// Replace `[start, end)` of node [i] with [text] linked to [href] (both
  /// the label and the target may change — the hover toolbar's "edit link").
  /// Existing marks are shifted across the replacement; the caret lands after
  /// the link.
  void replaceLink(int i, int start, int end, String text, String href) {
    if (i < 0 || i >= nodes.length) return;
    final node = nodes[i];
    if (node.isAtomic || node.kind == 'code_block' || node.kind == 'table') {
      return;
    }
    final s = start.clamp(0, node.text.length);
    final e = end.clamp(s, node.text.length);
    final newText = node.text.substring(0, s) + text + node.text.substring(e);
    final delta = text.length - (e - s);
    final shifted = <Mark>[];
    for (final m in marksFromData(node.data)) {
      var ms = m.start, me = m.end;
      if (ms >= e) {
        ms += delta;
      } else if (ms > s) {
        ms = s;
      }
      if (me > e) {
        me += delta;
      } else if (me > s) {
        me = s;
      }
      if (me > ms) shifted.add(Mark(ms, me, m.type, href: m.href));
    }
    final next =
        applyMark(shifted, s, s + text.length, 'link', href: href, add: true);
    node
      ..text = newText
      ..data = {...node.data, 'marks': marksToJson(next)};
    _dirty.remove(node.id);
    _sendNow([
      {
        'type': 'update_block',
        'block_id': node.id,
        'text': newText,
        'data': node.data,
      },
    ]);
    collapseTo(DocPosition(i, s + text.length));
  }

  /// Insert [text] at the caret (replacing any ranged selection), carrying
  /// [spans] — marks whose offsets are relative to [text]. Existing marks shift
  /// across the replacement; the caret lands after the inserted run. Used to
  /// paste inline content (e.g. inline math `$…$`) into the text flow without
  /// breaking the line.
  void insertInlineSpan(int i, int start, int end, String text, List<Mark> spans) {
    if (i < 0 || i >= nodes.length) return;
    final node = nodes[i];
    if (node.isAtomic || node.kind == 'code_block' || node.kind == 'table') {
      return;
    }
    final s = start.clamp(0, node.text.length);
    final e = end.clamp(s, node.text.length);
    final newText = node.text.substring(0, s) + text + node.text.substring(e);
    final delta = text.length - (e - s);
    final next = <Mark>[];
    // Shift existing marks across the [s, e) → text replacement (same rule as
    // replaceLink: marks inside the replaced range collapse to its start).
    for (final m in marksFromData(node.data)) {
      var ms = m.start, me = m.end;
      if (ms >= e) {
        ms += delta;
      } else if (ms > s) {
        ms = s;
      }
      if (me > e) {
        me += delta;
      } else if (me > s) {
        me = s;
      }
      if (me > ms) {
        next.add(Mark(ms, me, m.type, href: m.href, title: m.title));
      }
    }
    // Add the inserted spans, offset to the insertion point.
    for (final m in spans) {
      next.add(Mark(s + m.start, s + m.end, m.type, href: m.href, title: m.title));
    }
    node
      ..text = newText
      ..data = {...node.data, 'marks': marksToJson(next)};
    _dirty.remove(node.id);
    _sendNow([
      {
        'type': 'update_block',
        'block_id': node.id,
        'text': newText,
        'data': node.data,
      },
    ]);
    collapseTo(DocPosition(i, s + text.length));
  }

  /// Insert a paragraph as the new first block (Enter in the page title
  /// pushes the body down). Caret lands at its start.
  /// Insert an empty paragraph right after [index] and put the caret in it —
  /// how ↓ exits a table that is the document's last block.
  void insertParagraphAfter(int index) {
    if (index < 0 || index >= nodes.length) return;
    final created = EditorNode(id: _genId(), kind: 'paragraph', text: '');
    nodes.insert(index + 1, created);
    _sendNow([_insertOp(created, index + 1)]);
    collapseTo(DocPosition(index + 1, 0));
  }

  void insertParagraphAtTop(String text) {
    final created = EditorNode(id: _genId(), kind: 'paragraph', text: text);
    nodes.insert(0, created);
    _sendNow([_insertOp(created, 0)]);
    collapseTo(const DocPosition(0, 0));
  }

  /// Ensure a place to type when the document is empty.
  EditorNode ensureNotEmpty() {
    if (nodes.isNotEmpty) return nodes.first;
    final node = EditorNode(id: _genId(), kind: 'paragraph', text: '');
    nodes.add(node);
    _sendNow([_insertOp(node, 0)]);
    collapseTo(const DocPosition(0, 0));
    return node;
  }

  /// Insert AI-generated (or pasted) blocks after the focused node. If the
  /// focused node is an empty paragraph it is filled with the first block so no
  /// blank line is left behind. The caret moves to the end of the last block.
  void insertBlocksAfterFocus(
    List<({String kind, String text, Map<String, dynamic> data})> specs,
  ) {
    if (specs.isEmpty) return;
    final sel = selection;
    var focusIndex = (sel != null && sel.focus.node < nodes.length)
        ? sel.focus.node
        : nodes.length - 1;
    if (focusIndex < 0) focusIndex = 0;

    final ops = <DocOp>[];
    var first = 0;

    if (focusIndex < nodes.length &&
        nodes[focusIndex].text.isEmpty &&
        nodes[focusIndex].kind == 'paragraph') {
      final spec = specs.first;
      final node = nodes[focusIndex]
        ..kind = spec.kind
        ..text = spec.text
        ..data = Map<String, dynamic>.from(spec.data);
      _dirty.remove(node.id);
      ops.add({
        'type': 'update_block',
        'block_id': node.id,
        'kind': node.kind,
        'data': node.data,
        'text': node.text,
      });
      first = 1;
    }

    var insertAt = focusIndex + 1;
    for (var k = first; k < specs.length; k++) {
      final spec = specs[k];
      final node = EditorNode(
        id: _genId(),
        kind: spec.kind,
        text: spec.text,
        data: Map<String, dynamic>.from(spec.data),
      );
      nodes.insert(insertAt, node);
      ops.add(_insertOp(node, insertAt));
      insertAt++;
    }

    if (ops.isNotEmpty) _sendNow(ops);
    final lastIndex = (insertAt - 1).clamp(0, nodes.length - 1);
    collapseTo(DocPosition(lastIndex, nodes[lastIndex].text.length));
  }

  /// Paste [specs] as blocks, replacing any ranged selection first — so Ctrl+A →
  /// paste swaps the whole document instead of appending to it. With a collapsed
  /// caret this is exactly [insertBlocksAfterFocus]. Over a selection it deletes
  /// the selection, then: if that leaves an empty block (a whole-block or
  /// select-all paste) it overwrites that block via [replaceFocusedWithBlocks] —
  /// which also repairs its kind (e.g. a leftover empty heading); a partial
  /// selection keeps its surrounding text, so the paste is appended after it.
  void insertBlocksReplacingSelection(
    List<({String kind, String text, Map<String, dynamic> data})> specs,
  ) {
    if (specs.isEmpty) return;
    final sel = selection;
    if (sel != null && !sel.isCollapsed) {
      deleteSelection();
      final leftover = focusedNode;
      if (leftover != null && leftover.text.isEmpty) {
        replaceFocusedWithBlocks(specs);
        return;
      }
    }
    insertBlocksAfterFocus(specs);
  }

  /// Insert raw text at the caret within the focused node (replacing any
  /// in-node selection), preserving newlines. Used to paste into a code block so
  /// the content stays inside it.
  void insertTextAtCaret(String text) {
    final sel = selection;
    if (sel == null) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;
    final node = nodes[i];
    if (node.isAtomic || node.kind == 'table') {
      // Atomic blocks have no caret-editable text — splicing into node.text
      // silently corrupted an image's alt / a math block's LaTeX / a table's
      // dead text. Route the paste to a fresh paragraph below instead.
      insertBlocksAfterFocus([(kind: 'paragraph', text: text, data: <String, dynamic>{})]);
      return;
    }
    final len = node.text.length;
    final from = (sel.start.node == i ? sel.start.offset : 0).clamp(0, len);
    final to = (sel.end.node == i ? sel.end.offset : len).clamp(0, len);
    node.text = node.text.substring(0, from) + text + node.text.substring(to);
    _dirty.remove(node.id);
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'text': node.text},
    ]);
    collapseTo(DocPosition(i, from + text.length));
  }

  /// Replace the focused node with parsed blocks (used for multi-line Markdown
  /// paste): the first block overwrites the focused node, the rest are inserted
  /// after it. The caret lands at the end of the last block.
  void replaceFocusedWithBlocks(
    List<({String kind, String text, Map<String, dynamic> data})> specs,
  ) {
    if (specs.isEmpty) return;
    final sel = selection;
    if (sel == null) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;

    final node = nodes[i];
    final first = specs.first;
    node
      ..kind = first.kind
      ..text = first.text
      ..data = Map<String, dynamic>.from(first.data);
    _dirty.remove(node.id);
    final ops = <DocOp>[
      {
        'type': 'update_block',
        'block_id': node.id,
        'kind': node.kind,
        'data': node.data,
        'text': node.text,
      },
    ];

    var insertAt = i + 1;
    for (var k = 1; k < specs.length; k++) {
      final spec = specs[k];
      final created = EditorNode(
        id: _genId(),
        kind: spec.kind,
        text: spec.text,
        data: Map<String, dynamic>.from(spec.data),
      );
      nodes.insert(insertAt, created);
      ops.add(_insertOp(created, insertAt));
      insertAt++;
    }

    _sendNow(ops);
    final last = (insertAt - 1).clamp(0, nodes.length - 1);
    collapseTo(DocPosition(last, nodes[last].text.length));
  }

  /// Insert a divider (horizontal rule) at the caret. An empty focused
  /// paragraph becomes the divider; otherwise a divider is inserted after the
  /// focused node. A trailing paragraph is ensured so the caret has a text node
  /// to land on (the divider itself is atomic / non-focusable).
  void insertDivider() {
    final sel = selection;
    if (sel == null) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;
    final node = nodes[i];
    final ops = <DocOp>[];
    int dividerIndex;
    if (node.text.isEmpty && node.kind == 'paragraph') {
      node
        ..kind = 'divider'
        ..text = ''
        ..data = {};
      _dirty.remove(node.id);
      ops.add({
        'type': 'update_block',
        'block_id': node.id,
        'kind': 'divider',
        'text': '',
        'data': <String, dynamic>{},
      });
      dividerIndex = i;
    } else {
      final d = EditorNode(id: _genId(), kind: 'divider', text: '');
      nodes.insert(i + 1, d);
      ops.add(_insertOp(d, i + 1));
      dividerIndex = i + 1;
    }

    final afterIndex = dividerIndex + 1;
    if (afterIndex >= nodes.length || nodes[afterIndex].isAtomic) {
      final p = EditorNode(id: _genId(), kind: 'paragraph', text: '');
      nodes.insert(afterIndex, p);
      ops.add(_insertOp(p, afterIndex));
    }
    _sendNow(ops);
    collapseTo(DocPosition(afterIndex, 0));
  }

  /// Insert an image block (atomic) at the caret. Normally it carries our
  /// storage [fileId] + original [name]; pass [url] instead to reference an
  /// external image (the fallback when server-side re-hosting failed — the
  /// renderer loads a `url` block directly). Like [insertDivider], an empty
  /// focused paragraph becomes the image; otherwise it is inserted after the
  /// focused node, and a trailing paragraph is ensured for the caret.
  void insertImage({
    String? fileId,
    String? name,
    String? url,
    String alt = '',
    String? align,
  }) {
    assert((fileId != null) != (url != null), 'pass exactly one of fileId/url');
    final sel = selection;
    if (sel == null) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;
    final data = <String, dynamic>{
      if (fileId != null) 'file_id': fileId,
      if (url != null) 'url': url,
      'name': name ?? (url == null ? '' : url.split('/').last),
      'align': ?align,
    };
    final node = nodes[i];
    final ops = <DocOp>[];
    int imageIndex;
    if (node.text.isEmpty && node.kind == 'paragraph') {
      node
        ..kind = 'image'
        ..text = alt
        ..data = data;
      _dirty.remove(node.id);
      ops.add({
        'type': 'update_block',
        'block_id': node.id,
        'kind': 'image',
        'text': alt,
        'data': data,
      });
      imageIndex = i;
    } else {
      final created = EditorNode(id: _genId(), kind: 'image', text: alt, data: data);
      nodes.insert(i + 1, created);
      ops.add(_insertOp(created, i + 1));
      imageIndex = i + 1;
    }

    final afterIndex = imageIndex + 1;
    if (afterIndex >= nodes.length || nodes[afterIndex].isAtomic) {
      final p = EditorNode(id: _genId(), kind: 'paragraph', text: '');
      nodes.insert(afterIndex, p);
      ops.add(_insertOp(p, afterIndex));
    }
    _sendNow(ops);
    collapseTo(DocPosition(afterIndex, 0));
  }

  /// Replace an image's external `url` with our own `file_id` + `name` (after
  /// re-hosting). Looked up by node id since indices may have shifted. No-op if
  /// the node is gone or already has this file_id.
  void setImageSource(String nodeId, {required String fileId, required String name}) {
    final node = nodes.where((n) => n.id == nodeId).firstOrNull;
    if (node == null || node.kind != 'image') return;
    if (node.data['file_id'] == fileId) return;
    final data = {...node.data, 'file_id': fileId, 'name': name}..remove('url');
    node.data = data;
    _dirty.remove(node.id);
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  /// Set an image block's alignment (`left`/`center`/`right`).
  /// Point an image block at an external [url] (dropping any stored file_id) —
  /// the "replace with a link" path. When re-hosting is on the caller then runs
  /// the usual ladder, which swaps it back to a file_id if it can.
  void setImageUrl(String nodeId, String url) {
    final node = nodes.where((n) => n.id == nodeId).firstOrNull;
    if (node == null || node.kind != 'image') return;
    final name = Uri.tryParse(url)?.pathSegments.lastOrNull ?? '';
    final data = {...node.data, 'url': url, 'name': name}..remove('file_id');
    node.data = data;
    _dirty.remove(node.id);
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  void setImageAlign(int index, String align) {
    if (index < 0 || index >= nodes.length) return;
    final node = nodes[index];
    if (node.kind != 'image') return;
    node.data = {...node.data, 'align': align};
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  /// Live preview of an image's display width during a drag (no op sent).
  void previewImageWidth(int index, double width) {
    if (index < 0 || index >= nodes.length) return;
    final node = nodes[index];
    if (node.kind != 'image') return;
    node.data = {...node.data, 'width': width};
    notifyListeners();
  }

  /// Commit an image's display width.
  void setImageWidth(int index, double width) {
    if (index < 0 || index >= nodes.length) return;
    final node = nodes[index];
    if (node.kind != 'image') return;
    node.data = {...node.data, 'width': width};
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'data': node.data},
    ]);
    notifyListeners();
  }

  /// Append an empty paragraph after the last node and move the caret into it.
  /// Used as the downward escape from a trailing code block.
  void addParagraphAfterLast() {
    final node = EditorNode(id: _genId(), kind: 'paragraph', text: '');
    nodes.add(node);
    _sendNow([_insertOp(node, nodes.length - 1)]);
    collapseTo(DocPosition(nodes.length - 1, 0));
  }

  /// Clicking below the last line: focus its end, or append a paragraph when the
  /// last node already has content.
  void appendOrFocusLast() {
    if (nodes.isEmpty) {
      ensureNotEmpty();
      return;
    }
    final last = nodes[nodes.length - 1];
    // Already an empty paragraph to write in — just land there (don't stack
    // empty lines). Otherwise create a fresh paragraph so clicking the blank
    // area always drops you into normal prose, escaping a trailing
    // code block / heading / list item.
    if (last.kind == 'paragraph' && last.text.isEmpty) {
      collapseTo(DocPosition(nodes.length - 1, 0));
      return;
    }
    addParagraphAfterLast();
  }

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  bool _continuesOnEnter(String kind) =>
      kind == 'bulleted_list' ||
      kind == 'numbered_list' ||
      kind == 'todo' ||
      // Enter inside a quote stays in the quote (carrying its depth); a second
      // Enter on an empty quote line exits to a paragraph, like the lists.
      kind == 'quote';

  /// Replace a block's text wholesale (math source editing etc.).
  void setBlockText(String id, String text) {
    final node = nodes.where((n) => n.id == id).firstOrNull;
    if (node == null || node.text == text) return;
    node.text = text;
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'text': text, 'data': node.data},
    ]);
    notifyListeners();
  }

  /// Move the block at [from] to insertion [index] (0..nodes.length) — the
  /// gutter drag handle. Emits a `move_block` op; undo snapshots apply.
  bool moveBlock(int from, int index) {
    if (from < 0 || from >= nodes.length) return false;
    final to = index.clamp(0, nodes.length);
    if (to == from || to == from + 1) return false;
    final node = nodes.removeAt(from);
    final at = to > from ? to - 1 : to;
    nodes.insert(at, node);
    _sendNow([
      {
        'type': 'move_block',
        'block_id': node.id,
        'parent_id': rootBlockId,
        'index': at,
      },
    ]);
    if (!node.isAtomic) {
      setSelection(DocSelection.collapsed(DocPosition(at, 0)));
    }
    notifyListeners();
    return true;
  }

  /// Kinds that can live INSIDE a list item as container children
  /// (`data.li`) — what the markdown importer produces.
  static bool _attachableKind(String kind) =>
      kind == 'code_block' ||
      kind == 'quote' ||
      kind == 'paragraph' ||
      kind == 'divider';

  /// The indent level of the list item that would own node [i] as a
  /// container child: the nearest item above, skipping that item's other
  /// children; null when nothing is there to attach to.
  int? _owningItemIndentAt(int i) {
    for (var p = i - 1; p >= 0; p--) {
      final n = nodes[p];
      if (n.isListKind) return n.indent;
      if (n.liLevel != null) continue; // earlier children of the same item
      return null;
    }
    return null;
  }

  /// Can the block become a container child of a preceding list item (Tab)?
  bool canAttachToItem(String id) {
    final i = nodes.indexWhere((n) => n.id == id);
    if (i < 0) return false;
    final node = nodes[i];
    return !node.isListKind &&
        node.liLevel == null &&
        _attachableKind(node.kind) &&
        _owningItemIndentAt(i) != null;
  }

  /// Indent (+1) / outdent (-1) the list/todo items covered by the selection
  /// — Tab / Shift+Tab. Each item clamps to one level deeper than the
  /// nearest list item above it (no orphan levels). Non-list blocks attach
  /// to / detach from the preceding list item as container children
  /// (`data.li`). Returns true when anything changed.
  bool indentSelection(int delta) {
    final sel = selection;
    if (sel == null || nodes.isEmpty) return false;
    final lo = sel.start.node.clamp(0, nodes.length - 1);
    final hi = sel.end.node.clamp(0, nodes.length - 1);
    final ops = <DocOp>[];
    for (var i = lo; i <= hi; i++) {
      final node = nodes[i];
      if (!node.isListKind) {
        if (!_attachableKind(node.kind)) continue;
        if (delta > 0 && node.liLevel == null) {
          final owner = _owningItemIndentAt(i);
          if (owner == null) continue;
          final data = {...node.data}..['li'] = owner;
          node.data = data;
          ops.add({
            'type': 'update_block',
            'block_id': node.id,
            'text': node.text,
            'data': data,
          });
        } else if (delta < 0 && node.liLevel != null) {
          final data = {...node.data}..remove('li');
          node.data = data;
          ops.add({
            'type': 'update_block',
            'block_id': node.id,
            'text': node.text,
            'data': data,
          });
        }
        continue;
      }
      var maxIndent = 0;
      for (var p = i - 1; p >= 0; p--) {
        if (nodes[p].isListKind) {
          maxIndent = nodes[p].indent + 1;
          break;
        }
        // The item above may carry container children between it and us.
        if (nodes[p].liLevel != null) continue;
        break; // any other block above caps this item at level 0
      }
      final next = (node.indent + delta).clamp(0, maxIndent);
      if (next == node.indent) continue;
      final data = {...node.data};
      if (next > 0) {
        data['indent'] = next;
      } else {
        data.remove('indent');
      }
      node.data = data;
      ops.add({
        'type': 'update_block',
        'block_id': node.id,
        'text': node.text,
        'data': data,
      });
    }
    if (ops.isEmpty) return false;
    _sendNow(ops);
    notifyListeners();
    return true;
  }

  Map<String, dynamic> _continuationData(EditorNode node) {
    if (node.kind == 'todo') {
      return {'checked': false, if (node.indent > 0) 'indent': node.indent};
    }
    return Map<String, dynamic>.from(node.data)
      // The continuation stays in the SAME quote group: inheriting the
      // group-opener's qbreak visually severed the quote bar right where
      // Enter was pressed (Typora semantics: only Enter on an EMPTY quote
      // line leaves the group).
      ..remove('qbreak')
      // The split recomputes each half's marks; the parent's full set must
      // not ride along.
      ..remove('marks');
  }

  DocOp _insertOp(EditorNode node, int index) => {
    'type': 'insert_block',
    'parent_id': rootBlockId,
    'index': index,
    'block': {
      'id': node.id,
      'type': node.kind,
      'text': node.text,
      'data': node.data,
      'children': <String>[],
    },
  };

  /// Structural send: flush pending text first so order is correct.
  void _sendNow(List<DocOp> ops) {
    _saveTimer?.cancel();
    _saveTimer = null;
    final pending = <DocOp>[];
    if (_dirty.isNotEmpty) {
      for (final id in _dirty.toList()) {
        final node = nodes.where((n) => n.id == id).firstOrNull;
        // Skip ids already represented in this batch to avoid double-writes.
        final inBatch = ops.any((o) => o['block_id'] == id);
        if (node != null && !inBatch) {
          pending.add({'type': 'update_block', 'block_id': id, 'text': node.text});
        }
      }
      _dirty.clear();
    }
    _send([...pending, ...ops]);
  }

  Future<void> _send(List<DocOp> ops) {
    // Record the pre-change document before this committed mutation, unless we
    // are mid-restore (undo/redo emit their own diff ops through here).
    if (!_restoring) _recordHistory();
    final done = _chain.then((_) => onOps(ops)).catchError((_) {});
    _chain = done;
    return done;
  }

  // ---------------------------------------------------------------------------
  // Undo / redo
  // ---------------------------------------------------------------------------

  bool get canUndo => _undoStack.isNotEmpty || _dirty.isNotEmpty;
  bool get canRedo => _redoStack.isNotEmpty;

  /// Push the prior committed state and adopt the just-applied one as current.
  void _recordHistory() {
    _undoStack.add(_present);
    if (_undoStack.length > _maxHistory) _undoStack.removeAt(0);
    _redoStack.clear();
    _present = _snapshot();
  }

  _DocSnapshot _snapshot() => _DocSnapshot(
    nodes: [for (final n in nodes) n.copy()],
    selection: selection,
  );

  void undo() {
    // Commit any *pending* debounced typing first so its burst is its own undo
    // step (and restorable via redo), then step back. A non-null timer means
    // there are uncommitted edits; ids that linger in `_dirty` after a flush are
    // merely in-flight (already recorded) and must not be re-sent here.
    if (_saveTimer != null) flushPending();
    if (_undoStack.isEmpty) return;
    final target = _undoStack.removeLast();
    _redoStack.add(_present);
    _restore(target);
  }

  void redo() {
    if (_redoStack.isEmpty) return;
    final target = _redoStack.removeLast();
    _undoStack.add(_present);
    _restore(target);
  }

  /// Replace the live document with [target] and send the block-op diff so the
  /// backend follows. Does not touch the history stacks (the caller did).
  void _restore(_DocSnapshot target) {
    _restoring = true;
    final ops = _diffOps(nodes, target.nodes);
    nodes
      ..clear()
      ..addAll(target.nodes.map((n) => n.copy()));
    selection = target.selection;
    _clampSelection();
    // The initial-load baseline may have no selection; keep a caret so editing
    // can resume immediately after undoing all the way back.
    if (selection == null && nodes.isNotEmpty) {
      selection = const DocSelection.collapsed(DocPosition(0, 0));
    }
    _dirty.clear();
    if (ops.isNotEmpty) _send(ops);
    _present = _snapshot();
    _restoring = false;
    notifyListeners();
  }

  /// Block-op diff transforming the [from] document into [to]. The editor never
  /// reorders existing blocks, so deletes + position-indexed inserts + content
  /// updates fully reconstruct order (no `move_block` needed).
  List<DocOp> _diffOps(List<EditorNode> from, List<EditorNode> to) {
    final fromById = {for (final n in from) n.id: n};
    final toIds = {for (final n in to) n.id};
    final ops = <DocOp>[];
    for (final n in from) {
      if (!toIds.contains(n.id)) {
        ops.add({'type': 'delete_block', 'block_id': n.id});
      }
    }
    for (var index = 0; index < to.length; index++) {
      final n = to[index];
      final old = fromById[n.id];
      if (old == null) {
        ops.add(_insertOp(n, index));
      } else if (old.kind != n.kind ||
          old.text != n.text ||
          !_jsonEq(old.data, n.data)) {
        ops.add({
          'type': 'update_block',
          'block_id': n.id,
          'kind': n.kind,
          'text': n.text,
          'data': n.data,
        });
      }
    }
    return ops;
  }

  static bool _jsonEq(Object? a, Object? b) {
    if (identical(a, b)) return true;
    if (a is Map && b is Map) {
      if (a.length != b.length) return false;
      for (final key in a.keys) {
        if (!b.containsKey(key) || !_jsonEq(a[key], b[key])) return false;
      }
      return true;
    }
    if (a is List && b is List) {
      if (a.length != b.length) return false;
      for (var i = 0; i < a.length; i++) {
        if (!_jsonEq(a[i], b[i])) return false;
      }
      return true;
    }
    return a == b;
  }

  String _genId() =>
      'block_${DateTime.now().microsecondsSinceEpoch}_${_idCounter++}';

  @override
  void dispose() {
    _saveTimer?.cancel();
    super.dispose();
  }
}

/// An immutable point-in-time copy of the document for the undo/redo stacks.
class _DocSnapshot {
  _DocSnapshot({required this.nodes, required this.selection});

  final List<EditorNode> nodes; // deep-copied; never mutated in place
  final DocSelection? selection; // DocSelection/DocPosition are immutable
}

extension _FirstOrNull<E> on Iterable<E> {
  E? get firstOrNull {
    final it = iterator;
    return it.moveNext() ? it.current : null;
  }
}
