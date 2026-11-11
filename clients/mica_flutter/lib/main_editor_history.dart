part of 'main.dart';

extension _ShellEditorHistory on _WorkspaceShellState {
  ApplyHistory? _editorHistoryForDocument() {
    final local = _activeIsLocal;
    final documentId =
        (local ? _localBackend.bootstrap : _selectedBootstrap)?.document.id;
    if (documentId == null) return null;
    return (action) async {
      final current =
          (local ? _localBackend.bootstrap : _selectedBootstrap)?.document.id;
      if (!mounted || _activeIsLocal != local || current != documentId) {
        throw StateError('Editor document changed');
      }
      if (local) return _local.applyHistory(action);
      final session = await _syncCtrl.readyForDocument(documentId);
      if (!mounted ||
          _activeIsLocal ||
          _selectedBootstrap?.document.id != documentId) {
        throw StateError('Editor document changed');
      }
      return session.applyHistory(action);
    };
  }
}
