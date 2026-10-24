import 'model.dart';

/// Resolve nullable editor state before the renderer enters its layout loop.
/// Per-node layout uses scalar values, so no focus access can be hoisted out of
/// a conditional branch when selection is absent (Windows AOT startup crash).
({int node, int offset}) layoutCaret(DocSelection? selection) {
  if (selection == null || !selection.isCollapsed) return (node: -1, offset: 0);
  return (node: selection.focus.node, offset: selection.focus.offset);
}
