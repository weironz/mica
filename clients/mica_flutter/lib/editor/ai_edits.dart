import 'dart:convert';

import '../markdown/markdown_codec.dart' show BlockSpec;
import 'model.dart';

/// A frozen view of the editor body used to preview a proposed AI edit.
class EditorAiSnapshot {
  EditorAiSnapshot(this.rootBlockId, List<EditorNode> source)
    : nodes = List.unmodifiable([
        for (final node in source)
          EditorNode(
            id: node.id,
            kind: node.kind,
            text: node.text,
            data: Map<String, dynamic>.from(
              jsonDecode(jsonEncode(node.data)) as Map,
            ),
          ),
      ]);

  final String rootBlockId;
  final List<EditorNode> nodes;

  List<Map<String, dynamic>> get blocks => [
    for (final node in nodes)
      {
        'id': node.id,
        'type': node.kind,
        'text': node.text,
        'data': node.data,
        'children': <String>[],
      },
  ];
}

/// Parsed by the Rust preview endpoint; the editor only applies its block specs.
class AiBlockEdit {
  const AiBlockEdit({
    required this.blockIds,
    required this.before,
    required this.after,
    required this.replacement,
  });

  factory AiBlockEdit.fromJson(Map<String, dynamic> json) => AiBlockEdit(
    blockIds: List<String>.from(json['block_ids'] as List),
    before: json['before'] as String,
    after: json['after'] as String,
    replacement: [
      for (final block in json['replacement'] as List)
        (
          kind: block['type'] as String,
          text: block['text'] as String,
          data: Map<String, dynamic>.from(block['data'] as Map? ?? const {}),
        ),
    ],
  );

  final List<String> blockIds;
  final String before;
  final String after;
  final List<BlockSpec> replacement;
}

class AiEditPreview {
  const AiEditPreview(this.edits);

  factory AiEditPreview.fromJson(Map<String, dynamic> json) => AiEditPreview([
    for (final edit in json['edits'] as List)
      AiBlockEdit.fromJson(Map<String, dynamic>.from(edit as Map)),
  ]);

  final List<AiBlockEdit> edits;
}

class AiEditConflict implements Exception {
  const AiEditConflict();
}

class AiEditSaveFailure implements Exception {
  const AiEditSaveFailure();
}
