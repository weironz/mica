part of 'controller.dart';

class _HistoryEntry {
  _HistoryEntry(this.inverse, this.before, this.after);
  final OperationInverse? inverse;
  final HistorySelection before;
  HistorySelection after;
}

/// Only the focused blocks typed into while an async history command waits.
class _HistoryDraft {
  _HistoryDraft(this.before) : text = before.text;
  final EditorNode before;
  String text;
  DocSelection? selection;
}

extension _ControllerHistory on EditorController {
  void _afterHistory(void Function() edit) {
    _historyFuture?.then((_) {
      if (!_disposed) edit();
    });
  }

  void _rememberHistorySelection(DocSelection? value) {
    if (value == null || _saveTimer != null || _restoring || _historyBusy) {
      return;
    }
    bool unchanged(DocPosition position) {
      final i = position.node;
      if (i < 0 || i >= nodes.length || i >= _historyBaseline.length) {
        return false;
      }
      final before = _historyBaseline[i];
      final now = nodes[i];
      return before.id == now.id &&
          before.kind == now.kind &&
          before.text == now.text;
    }

    if (unchanged(value.anchor) && unchanged(value.focus)) {
      _historySelection = value;
    }
  }

  void _finishHistorySelection() {
    final entry = _historyAwaitingSelection;
    if (entry == null) return;
    entry.after = HistorySelection(nodes, selection);
    _historyAwaitingSelection = null;
    _historySelection = selection;
  }

  void _adoptHistoryBaseline() {
    _historyBaseline = [for (final n in nodes) historyCopyNode(n)];
    _historySelection = selection;
  }

  void _historyFault(Object error) {
    if (_disposed) return;
    opFaultCount++;
    onOpFault?.call(error, opFaultCount);
  }

  _HistoryEntry? _recordHistory(List<DocOp> ops) {
    _historyAwaitingSelection = null;
    final inverse = OperationInverse.capture(_historyBaseline, nodes, ops);
    if (inverse.isEmpty) return null; // retries are not new edits
    final entry = _HistoryEntry(
      onHistory == null ? inverse : null,
      HistorySelection(_historyBaseline, _historySelection ?? selection),
      HistorySelection(nodes, selection),
    );
    _undoStack.add(entry);
    _historyAwaitingSelection = entry;
    if (onHistory == null && _undoStack.length > EditorController._maxHistory) {
      _undoStack.removeAt(0);
    }
    _redoStack.clear();
    _adoptHistoryBaseline();
    return entry;
  }

  Future<void> _stepHistory({required bool redo}) {
    if (_historyBusy || _disposed) return Future.value();
    _finishHistorySelection();
    if (onHistory != null) {
      _historyBusy = true;
      return _historyFuture = _stepEngineHistory(redo: redo);
    }
    // Preserve the fallback's synchronous restore for embedded editors.
    if (_saveTimer != null) flushPending();
    final source = redo ? _redoStack : _undoStack;
    if (source.isEmpty) return Future.value();
    final entry = source.removeLast();
    final restored = entry.inverse!.apply(nodes);
    final ops = _diffOps(nodes, restored);
    final reverse = OperationInverse.capture(nodes, restored, ops);
    (redo ? _undoStack : _redoStack).add(
      _HistoryEntry(reverse, entry.after, entry.before),
    );
    _restoring = true;
    _replaceFromHistory(restored, entry.before);
    final sent = ops.isNotEmpty ? _send(ops) : Future<bool>.value(true);
    _restoring = false;
    notifyListeners();
    return sent.then((_) {});
  }

  Future<void> _stepEngineHistory({required bool redo}) async {
    var applied = false;
    try {
      // Failed commits retain their dirty text. Do not move the engine's stack
      // until that local intent is durable, including already queued sends.
      if (!await _flushPending()) return;
      await _chain;
      if (_disposed) return;
      final source = redo ? _redoStack : _undoStack;
      if (source.isEmpty) return;
      final command = redo ? HistoryAction.redo : HistoryAction.undo;
      final state = await onHistory!(command);
      if (_disposed) return;
      final destination = redo ? _undoStack : _redoStack;
      final depth = redo ? state.redoDepth : state.undoDepth;
      final remaining = (depth ?? source.length - 1).clamp(0, source.length);
      final entry = source[remaining.clamp(0, source.length - 1)];
      source.removeRange(remaining, source.length);
      final oppositeDepth = redo ? state.undoDepth : state.redoDepth;
      final changed =
          oppositeDepth == null || oppositeDepth > destination.length;
      final caret = changed ? entry.before : HistorySelection(nodes, selection);
      if (changed) {
        destination.add(_HistoryEntry(null, entry.after, entry.before));
      }
      if (!state.canUndo) _undoStack.clear();
      if (!state.canRedo) _redoStack.clear();
      _replaceFromHistory(state.nodes, caret);
      applied = true;
      final error = state.persistenceError;
      if (error != null) _historyFault(error);
    } catch (error) {
      // A thrown callback did not execute the command; its metadata stays put.
      _historyFault(error);
    } finally {
      _historyBusy = false;
      _historyFuture = null;
      final drafts = _historyDrafts.values.toList();
      _historyDrafts.clear();
      if (!_disposed) {
        for (final draft in drafts) {
          if (applied) {
            _replayHistoryDraft(draft);
          } else if (nodes.any((n) => n.id == draft.before.id)) {
            // No canonical replacement occurred: the live preview already
            // contains this draft, and only its pending flag was deferred.
            _markDirty(draft.before.id);
          }
        }
        notifyListeners();
      }
    }
  }

  void _replayHistoryDraft(_HistoryDraft draft) {
    final before = draft.before.text;
    if (before == draft.text || nodes.isEmpty) return;
    var start = _commonPrefix(before, draft.text);
    // Never split a UTF-16 surrogate when two emoji share the high surrogate.
    if (start > 0 &&
        start < before.length &&
        before.codeUnitAt(start) >= 0xdc00 &&
        before.codeUnitAt(start) <= 0xdfff) {
      start--;
    }
    final suffix = _commonSuffix(before, draft.text, start);
    var oldEnd = before.length - suffix;
    var newEnd = draft.text.length - suffix;
    if (oldEnd < before.length &&
        oldEnd > 0 &&
        before.codeUnitAt(oldEnd) >= 0xdc00 &&
        before.codeUnitAt(oldEnd) <= 0xdfff) {
      oldEnd++;
      newEnd++;
    }
    var index = nodes.indexWhere((n) => n.id == draft.before.id);
    int from, to;
    if (index >= 0) {
      final mapped = ReconcileSelection(
        [draft.before],
        DocSelection(
          anchor: DocPosition(0, start),
          focus: DocPosition(0, oldEnd),
        ),
      ).remap([nodes[index]])!;
      from = mapped.start.offset;
      to = mapped.end.offset;
    } else {
      // Undoing an inserted block removes its identity. New input belongs at
      // the restored caret, rather than resurrecting the whole removed block.
      index = (selection?.focus.node ?? 0).clamp(0, nodes.length - 1);
      from = (selection?.focus.offset ?? 0).clamp(0, nodes[index].text.length);
      to = from;
    }
    final node = nodes[index];
    final next = node.text.replaceRange(
      from,
      to,
      draft.text.substring(start, newEnd),
    );
    final typed = draft.before.copy()..text = draft.text;
    final restored = node.copy()..text = next;
    final mappedCaret = ReconcileSelection([
      typed,
    ], draft.selection).remap([restored]);
    selection = DocSelection(
      anchor: DocPosition(index, from),
      focus: DocPosition(index, to),
    );
    setFocusedText(
      next,
      mappedCaret?.anchor.offset ?? from + newEnd - start,
      mappedCaret?.focus.offset ?? from + newEnd - start,
    );
  }

  void _replaceFromHistory(List<EditorNode> restored, HistorySelection caret) {
    _historyAwaitingSelection = null;
    nodes
      ..clear()
      ..addAll(restored.map((n) => n.copy()));
    selection = caret.restore(nodes);
    _dirty.clear();
    _adoptHistoryBaseline();
  }
}
