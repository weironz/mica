part of 'main.dart';

extension _ShellEditorHistory on _WorkspaceShellState {
  ApplyHistory? _editorHistoryForDocument() {
    final local = _activeIsLocal;
    final documentId =
        (local ? _localBootstrap : _selectedBootstrap)?.document.id;
    if (documentId == null) return null;
    return (action) async {
      final current =
          (local ? _localBootstrap : _selectedBootstrap)?.document.id;
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

List<EditorNode> _editorNodes(Iterable<DocumentBlock> blocks) => [
  for (final b in blocks)
    EditorNode(
      id: b.id,
      kind: b.kind,
      text: b.text,
      data: Map<String, dynamic>.from(b.data),
    ),
];

List<({String blockId, int offset, Color color, String label})> _editorCursors(
  Iterable<PresenceUser> presence,
) => [
  for (final p in presence)
    if (p.hasCursor)
      (
        blockId: p.cursorBlockId!,
        offset: p.cursorOffset!,
        color: p.color,
        label: p.name,
      ),
];
