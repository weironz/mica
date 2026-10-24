import 'marks.dart';
import 'model.dart';

typedef InlineSelectionRange = ({EditorNode node, int start, int end});
typedef InlineMarkChange = ({EditorNode node, Map<String, dynamic> data});

/// Inline-text intersections only: list structure and atomic/code content stay
/// outside formatting, even when the document selection crosses those blocks.
Iterable<InlineSelectionRange> inlineSelectionRanges(
  List<EditorNode> nodes,
  DocSelection? selection,
) sync* {
  if (selection == null || selection.isCollapsed) return;
  final first = selection.start;
  final last = selection.end;
  if (first.node < 0 || last.node >= nodes.length) return;
  for (var i = first.node; i <= last.node; i++) {
    final node = nodes[i];
    if (!EditorNode.isInlineTextKind(node.kind)) continue;
    final start = (i == first.node ? first.offset : 0).clamp(
      0,
      node.text.length,
    );
    final end = (i == last.node ? last.offset : node.text.length).clamp(
      0,
      node.text.length,
    );
    if (start < end) yield (node: node, start: start, end: end);
  }
}

bool inlineSelectionHasMark(
  List<EditorNode> nodes,
  DocSelection? selection,
  String type,
) {
  final ranges = inlineSelectionRanges(nodes, selection).toList();
  return ranges.isNotEmpty &&
      ranges.every(
        (range) => rangeHasMark(
          marksFromData(range.node.data),
          range.start,
          range.end,
          type,
        ),
      );
}

/// Decide once for the entire selection: remove only if every selected text
/// range already has the mark. Mixed selections become uniformly formatted.
List<InlineMarkChange> toggleSelectionMark(
  List<EditorNode> nodes,
  DocSelection? selection,
  String type, {
  String? href,
}) {
  final ranges = inlineSelectionRanges(nodes, selection).toList();
  if (ranges.isEmpty) return const [];
  final add = !ranges.every(
    (range) => rangeHasMark(
      marksFromData(range.node.data),
      range.start,
      range.end,
      type,
    ),
  );
  if (type == 'link' && add && (href == null || href.isEmpty)) return const [];
  return [
    for (final range in ranges)
      (
        node: range.node,
        data: {
          ...range.node.data,
          'marks': marksToJson(
            applyMark(
              marksFromData(range.node.data),
              range.start,
              range.end,
              type,
              href: href,
              add: add,
            ),
          ),
        },
      ),
  ];
}
