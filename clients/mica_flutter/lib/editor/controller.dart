import 'dart:async';

import 'package:flutter/foundation.dart';

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

  // ---------------------------------------------------------------------------
  // Loading & remote reconciliation
  // ---------------------------------------------------------------------------

  /// Replace the whole document (initial load). Does not emit ops.
  void load(List<EditorNode> next) {
    nodes
      ..clear()
      ..addAll(next.map((n) => n.copy()));
    _clampSelection();
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
    nodes[i].text = text;
    selection = DocSelection(
      anchor: DocPosition(i, selStart.clamp(0, text.length)),
      focus: DocPosition(i, selEnd.clamp(0, text.length)),
    );
    goalX = null;
    _markDirty(nodes[i].id);
    notifyListeners();
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
        ops.add({'type': 'update_block', 'block_id': id, 'text': node.text});
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

    final before = node.text.substring(0, at);
    final after = node.text.substring(at);
    final newKind = _continuesOnEnter(node.kind) ? node.kind : 'paragraph';
    final newData = _continuesOnEnter(node.kind)
        ? _continuationData(node)
        : <String, dynamic>{};
    final created = EditorNode(id: _genId(), kind: newKind, text: after, data: newData);

    node.text = before;
    nodes.insert(i + 1, created);
    _dirty.remove(node.id);
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'text': before},
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

    final continues = _continuesOnEnter(node.kind);
    final created = EditorNode(
      id: _genId(),
      kind: continues ? node.kind : 'paragraph',
      text: after,
      data: continues ? _continuationData(node) : <String, dynamic>{},
    );
    node.text = before;
    nodes.insert(i + 1, created);
    _dirty.remove(node.id);
    _sendNow([
      {'type': 'update_block', 'block_id': node.id, 'text': before},
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

    // Nothing before the first block to merge into.
    if (i == 0) return false;
    final prev = nodes[i - 1];
    final junction = prev.text.length;
    final merged = prev.text + cur.text;
    prev.text = merged;
    nodes.removeAt(i);
    _dirty
      ..remove(cur.id)
      ..remove(prev.id);
    _sendNow([
      {'type': 'update_block', 'block_id': prev.id, 'text': merged},
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
    if (i < 0 || i + 1 >= nodes.length) return false;
    final cur = nodes[i];
    final next = nodes[i + 1];
    final junction = cur.text.length;
    final merged = cur.text + next.text;
    cur.text = merged;
    nodes.removeAt(i + 1);
    _dirty
      ..remove(cur.id)
      ..remove(next.id);
    _sendNow([
      {'type': 'update_block', 'block_id': cur.id, 'text': merged},
      {'type': 'delete_block', 'block_id': next.id},
    ]);
    collapseTo(DocPosition(i, junction));
    return true;
  }

  /// Serialize the current ranged selection to text (tables become GFM). Empty
  /// when the selection is collapsed/absent.
  String selectionText() {
    final sel = selection;
    if (sel == null || sel.isCollapsed) return '';
    final s = sel.start;
    final e = sel.end;

    String nodeText(int i, int from, int to) {
      final node = nodes[i];
      if (node.kind == 'table') {
        return tableToMarkdown(TableData.fromBlock(node.data));
      }
      final len = node.text.length;
      return node.text.substring(from.clamp(0, len), to.clamp(0, len));
    }

    if (s.node == e.node) {
      return nodeText(s.node, s.offset, e.offset);
    }
    final parts = <String>[
      nodeText(s.node, s.offset, nodes[s.node].text.length),
      for (var i = s.node + 1; i < e.node; i++)
        nodeText(i, 0, nodes[i].text.length),
      nodeText(e.node, 0, e.offset),
    ];
    return parts.join('\n');
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
      _dirty.remove(node.id);
      _sendNow([
        {'type': 'update_block', 'block_id': node.id, 'text': node.text},
      ]);
      collapseTo(DocPosition(start.node, s));
      return true;
    }

    final startNode = nodes[start.node];
    final endNode = nodes[end.node];
    final s = start.offset.clamp(0, startNode.text.length);
    final e = end.offset.clamp(0, endNode.text.length);
    final merged = startNode.text.substring(0, s) + endNode.text.substring(e);
    final removed = [for (var i = start.node + 1; i <= end.node; i++) nodes[i].id];

    startNode.text = merged;
    nodes.removeRange(start.node + 1, end.node + 1);
    _dirty.remove(startNode.id);
    final ops = <DocOp>[
      {'type': 'update_block', 'block_id': startNode.id, 'text': merged},
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

  void setTableAlign(int index, String align) {
    final table = _tableAt(index);
    if (table == null) return;
    _writeTable(
      index,
      TableData(
        table.rows,
        header: table.header,
        align: align,
        widths: table.widths,
      ),
    );
  }

  /// Live preview of column widths during a drag (no op sent / no save).
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

    if (node.kind == 'paragraph') {
      for (var lvl = 6; lvl >= 1; lvl--) {
        final marker = '${'#' * lvl} ';
        if (caret == marker.length && text.startsWith(marker)) {
          return convert('heading', {'level': lvl}, marker.length);
        }
      }
      if (caret == 2 && (text.startsWith('- ') || text.startsWith('* '))) {
        return convert('bulleted_list', {}, 2);
      }
      if (caret == 2 && text.startsWith('> ')) {
        return convert('quote', {}, 2);
      }
      final numbered = RegExp(r'^\d+\. ').firstMatch(text);
      if (numbered != null && caret == numbered.end) {
        return convert('numbered_list', {}, numbered.end);
      }
      if (caret == 3 && text == '```') {
        return convert('code_block', {}, 3);
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
    return false;
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
    _sendNow([
      {
        'type': 'update_block',
        'block_id': node.id,
        'kind': kind,
        'data': node.data,
        'text': newText,
      },
    ]);
    collapseTo(DocPosition(i, s));
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

  /// Insert raw text at the caret within the focused node (replacing any
  /// in-node selection), preserving newlines. Used to paste into a code block so
  /// the content stays inside it.
  void insertTextAtCaret(String text) {
    final sel = selection;
    if (sel == null) return;
    final i = sel.focus.node;
    if (i >= nodes.length) return;
    final node = nodes[i];
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
      kind == 'bulleted_list' || kind == 'numbered_list' || kind == 'todo';

  Map<String, dynamic> _continuationData(EditorNode node) {
    if (node.kind == 'todo') return {'checked': false};
    return Map<String, dynamic>.from(node.data);
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
    final done = _chain.then((_) => onOps(ops)).catchError((_) {});
    _chain = done;
    return done;
  }

  String _genId() =>
      'block_${DateTime.now().microsecondsSinceEpoch}_${_idCounter++}';

  @override
  void dispose() {
    _saveTimer?.cancel();
    super.dispose();
  }
}

extension _FirstOrNull<E> on Iterable<E> {
  E? get firstOrNull {
    final it = iterator;
    return it.moveNext() ? it.current : null;
  }
}
