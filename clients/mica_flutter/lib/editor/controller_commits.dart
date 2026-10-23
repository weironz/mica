part of 'controller.dart';

class _PendingEditorCommit {
  _PendingEditorCommit(
    this.ops,
    this.history, {
    this.canSend,
    this.rollback,
    this.previousRedo,
  });

  final List<DocOp> ops;
  final _HistoryEntry? history;
  final bool Function()? canSend;
  final OperationInverse? rollback;
  final List<_HistoryEntry>? previousRedo;
  bool attempted = false;
  bool saved = false;
}

extension _ControllerCommits on EditorController {
  void _compoundEdit(void Function() edit) {
    if (_compoundOps != null) {
      edit();
      return;
    }
    final ops = _compoundOps = <DocOp>[];
    try {
      edit();
    } finally {
      _compoundOps = null;
      if (ops.isNotEmpty) _sendNow(ops);
    }
  }

  Future<bool> _retryPendingCommits() {
    final done = _chain.then((_) => _drainPendingCommits());
    _chain = done.then((_) {});
    return done;
  }

  Future<bool> _drainPendingCommits() async {
    while (_pendingCommits.isNotEmpty) {
      if (_disposed) return false;
      final batch = _pendingCommits.first;
      if (!batch.attempted && batch.canSend?.call() == false) {
        _pendingCommits.removeAt(0);
        _cancelUnsentCommit(batch);
        continue;
      }
      batch.attempted = true;
      try {
        // Retrying passes the SAME batch object. Real hosts acknowledge a batch
        // already mirrored into the engine by saving its retained diff only.
        await onOps(batch.ops);
      } catch (error) {
        _historyFault(error);
        _scheduleRetry();
        return false;
      }
      batch.saved = true;
      _pendingCommits.removeAt(0);
    }
    _retryAttempts = 0;
    _retryTimer?.cancel();
    _retryTimer = null;
    return true;
  }

  void _cancelUnsentCommit(_PendingEditorCommit batch) {
    final inverse = batch.rollback;
    if (inverse == null) return;
    final last =
        _undoStack.isNotEmpty && identical(_undoStack.last, batch.history);
    _undoStack.remove(batch.history);
    if (last && _redoStack.isEmpty && batch.previousRedo != null) {
      _redoStack.addAll(batch.previousRedo!);
    }
    if (identical(_historyAwaitingSelection, batch.history)) {
      _historyAwaitingSelection = null;
    }
    final caret = ReconcileSelection(nodes, selection);
    final restored = inverse.apply(nodes);
    nodes
      ..clear()
      ..addAll(restored);
    selection = caret.remap(nodes);
    _historyBaseline = inverse.apply(_historyBaseline);
    notifyListeners();
  }
}
