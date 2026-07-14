import 'package:flutter/widgets.dart';

import 'marks.dart';

/// A [TextEditingController] for a table cell that renders inline MARKS live
/// (WYSIWYG) instead of the raw Markdown source. Its [text] is the clean cell
/// text; [marks] holds the bold / italic / code / strike / link ranges; and
/// [buildTextSpan] paints them — so **bold** shows as bold WHILE you edit and the
/// `**` / `` ` `` delimiters never appear (unlike the old Typora-style source
/// field). Built from a cell's stored raw Markdown; [serialize] turns
/// (text, marks) back into Markdown for storage, so the cell schema and the GFM
/// round-trip are unchanged — only the editing surface becomes WYSIWYG.
///
/// Text edits are diffed (common prefix/suffix) so the mark ranges shift with
/// the text, reusing the same [shiftMarks] the main editor uses.
class CellEditController extends TextEditingController {
  CellEditController(String rawMarkdown) {
    final parsed = parseInline(rawMarkdown);
    marks = parsed.marks;
    // Seed the clean text. `_seeding` suppresses the edit-diff below: the marks
    // from parseInline already match this text, they must not be re-shifted.
    value = TextEditingValue(
      text: parsed.text,
      selection: TextSelection.collapsed(offset: parsed.text.length),
    );
    _seeding = false;
  }

  /// Inline marks over [text], mirroring the main editor's model.
  List<Mark> marks = const [];

  bool _seeding = true;

  /// The cell content as raw Markdown, for storage.
  String serialize() => inlineToMarkdown(text, marks);

  @override
  set value(TextEditingValue newValue) {
    if (!_seeding) {
      final old = super.value.text;
      final neu = newValue.text;
      if (neu != old) {
        // A single contiguous edit: everything between the shared prefix and
        // suffix was replaced. Shift the marks to track it.
        final prefix = _commonPrefix(old, neu);
        final suffix = _commonSuffix(old, neu, prefix);
        marks = shiftMarks(
          marks,
          prefix,
          old.length - suffix,
          neu.length - old.length,
          neu.length,
        );
      }
    }
    super.value = newValue;
  }

  /// Toggle inline [type] over the current selection (no-op when the selection
  /// is collapsed, or for a link with no [href]). Returns whether it applied.
  bool toggleMark(String type, {String? href}) {
    final sel = selection;
    if (!sel.isValid || sel.isCollapsed) return false;
    final from = sel.start;
    final to = sel.end;
    if (type == 'link' && (href == null || href.isEmpty)) return false;
    final has = rangeHasMark(marks, from, to, type);
    marks = applyMark(marks, from, to, type, href: href, add: !has);
    notifyListeners(); // repaint via buildTextSpan
    return true;
  }

  @override
  TextSpan buildTextSpan({
    required BuildContext context,
    TextStyle? style,
    required bool withComposing,
  }) {
    // Render the marks over the field's own style — this is what makes editing
    // WYSIWYG. The IME's composing region is left un-underlined (the candidate
    // window still positions correctly from the field's caret rect).
    return buildMarkedSpan(text, marks, style ?? const TextStyle());
  }

  int _commonPrefix(String a, String b) {
    final n = a.length < b.length ? a.length : b.length;
    var i = 0;
    while (i < n && a.codeUnitAt(i) == b.codeUnitAt(i)) {
      i++;
    }
    return i;
  }

  int _commonSuffix(String a, String b, int prefix) {
    final max = (a.length < b.length ? a.length : b.length) - prefix;
    var i = 0;
    while (i < max &&
        a.codeUnitAt(a.length - 1 - i) == b.codeUnitAt(b.length - 1 - i)) {
      i++;
    }
    return i;
  }
}
