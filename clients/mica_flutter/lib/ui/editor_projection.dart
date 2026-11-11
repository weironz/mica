import 'package:flutter/material.dart';
import '../api/models.dart';
import '../editor/editor.dart';

List<EditorNode> editorNodesFromBlocks(Iterable<DocumentBlock> blocks) => [
  for (final b in blocks)
    EditorNode(
      id: b.id,
      kind: b.kind,
      text: b.text,
      data: Map<String, dynamic>.from(b.data),
    ),
];

List<({String blockId, int offset, Color color, String label})>
editorCursorsFromPresence(Iterable<PresenceUser> presence) => [
  for (final p in presence)
    if (p.hasCursor)
      (
        blockId: p.cursorBlockId!,
        offset: p.cursorOffset!,
        color: p.color,
        label: p.name,
      ),
];
