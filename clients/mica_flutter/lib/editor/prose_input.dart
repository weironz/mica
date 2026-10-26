import 'package:flutter/services.dart';
import 'controller.dart';
import 'model.dart';
import '../web/text_diff.dart' show commonPrefix, commonSuffix;

/// Tracks the document range hidden by the platform's focus-block buffer.
/// Document mutations remain in EditorController.spliceRange.
class ProseInputSession {
  DocSelection? _range;
  String? _base;
  bool _composing = false;


  /// Drop a capture whose buffer no longer belongs to the platform session.
  void reset() {
    _range = null;
    _base = null;
    _composing = false;
  }
  /// False leaves normal in-block input to the caller. True consumes an edit
  /// replacing a cross-block selection, including its transient composition.
  bool apply(EditorController controller, TextEditingValue value) {
    if (controller.isApplyingHistory) return false;
    final node = controller.focusedNode;
    final selection = controller.selection;
    final range = _range ?? (selection?.isMultiNode == true ? selection : null);
    if (node == null || range == null) return false;
    _range = range;
    final base = _base ??= node.text;
    if (value.composing.isValid && !value.composing.isCollapsed) {
      _composing = true;
      controller.setFocusedText(value.text,
          value.selection.baseOffset < 0 ? value.text.length : value.selection.baseOffset,
          value.selection.extentOffset < 0 ? value.text.length : value.selection.extentOffset);
      return true;
    }
    final inserted = insertedText(base, value.text);
    final wasComposing = _composing;
    _range = null;
    _base = null;
    _composing = false;
    if (wasComposing && inserted.isEmpty && value.text == base) {
      controller.setFocusedText(base, range.focus.offset, range.focus.offset);
      controller.setSelection(range);
      return true;
    }
    // The preedit mirror changed the focus buffer; include that inserted tail
    // in the captured replacement so committing cannot duplicate the preedit.
    final focus = selection?.focus.node;
    // The captured START is a left boundary: at the preedit insertion point it
    // stays put, or a backward selection would strand the preedit as a prefix.
    // Only the range END shifts right across the mirrored preedit.
    DocPosition mapped(DocPosition p) => p.node != focus
        ? p
        : DocPosition(
            p.node,
            _mapOffset(p.offset, base, node.text,
                rightBias: p.compareTo(range.end) == 0),
          );
    return controller.spliceRange(
      DocSelection(anchor: mapped(range.anchor), focus: mapped(range.focus)), inserted);
  }
}

String insertedText(String old, String next, {DocSelection? selection, int? node}) {
  if (selection != null && !selection.isCollapsed &&
      selection.start.node == node && selection.end.node == node) {
    final start = selection.start.offset.clamp(0, old.length);
    final end = selection.end.offset.clamp(start, old.length);
    final tail = old.length - end;
    if (next.length >= start + tail && next.startsWith(old.substring(0, start)) &&
        next.endsWith(old.substring(end))) {
      return next.substring(start, next.length - tail);
    }
  }
  final prefix = commonPrefix(old, next);
  final suffix = commonSuffix(old, next, prefix);
  return next.substring(prefix, next.length - suffix);
}

int _mapOffset(int offset, String from, String to, {required bool rightBias}) {
  if (from == to) return offset;
  final prefix = commonPrefix(from, to);
  final suffix = commonSuffix(from, to, prefix);
  final oldEnd = from.length - suffix;
  if (rightBias) {
    if (offset < prefix) return offset;
    if (offset >= oldEnd) return offset + to.length - from.length;
  } else {
    if (offset <= prefix) return offset;
    if (offset > oldEnd) return offset + to.length - from.length;
  }
  return prefix;
}
