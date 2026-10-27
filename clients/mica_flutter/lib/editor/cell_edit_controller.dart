import 'package:flutter/widgets.dart';

import 'marks.dart';
import 'controller.dart';
import 'table.dart';
import '../markdown/markdown_codec.dart' show markdownCodec, MarkdownCodecException;
import '../ui/theme_tokens.dart';
import '../web/text_diff.dart' show commonPrefix, commonSuffix;

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

  /// HTML for the selected clean text, clipping the existing marks rather than
  /// re-parsing text as Markdown (which loses code and treats literal syntax
  /// as formatting). One paragraph block through the engine's Clipboard
  /// dialect — the cell editor is inline-only, so a paragraph is the whole
  /// grammar it needs.
  String selectionHtml() {
    final sel = selection;
    if (!sel.isValid || sel.isCollapsed) return '';
    try {
      final html = markdownCodec.renderClipboardHtml([
        (
          kind: 'paragraph',
          text: sel.textInside(text),
          data: {
            if (marks.isNotEmpty)
              'marks': marksToJson(clipMarks(marks, sel.start, sel.end)),
          },
        ),
      ]);
      // The cell editor's flavor is a BARE inline fragment — it lands inside a
      // table cell, where a <p> wrapper would be invalid nesting. The engine's
      // Clipboard dialect speaks blocks, so unwrap the one paragraph it wrote.
      return html.startsWith('<p>') && html.endsWith('</p>')
          ? html.substring(3, html.length - 4)
          : html;
    } on MarkdownCodecException {
      return '';
    }
  }

  @override
  set value(TextEditingValue newValue) {
    if (!_seeding) {
      final old = super.value.text;
      final neu = newValue.text;
      if (neu != old) {
        // A single contiguous edit: everything between the shared prefix and
        // suffix was replaced. Shift the marks to track it.
        final prefix = commonPrefix(old, neu);
        final suffix = commonSuffix(old, neu, prefix);
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
    return buildMarkedSpan(
      text,
      marks,
      style ?? const TextStyle(),
      MicaTheme.of(context),
    );
  }
}

/// Stable cell address and draft reconciliation, independent of the overlay.
class CellEditSession {
  CellEditSession(this.document, this.tableId, this.row, this.col, this.editor)
      : _opening = editor.serialize();
  final EditorController document;
  final String tableId;
  final int row, col;
  final CellEditController editor;
  String _opening;
  bool rebasing = false;

  int resolve() {
    final at = document.nodes.indexWhere((n) => n.id == tableId && n.kind == 'table');
    if (at < 0) return -1;
    final table = TableData.fromBlock(document.nodes[at].data);
    return row < 0 || row >= table.rows.length || col < 0 || col >= table.rows[row].length ? -1 : at;
  }

  void preview() {
    final at = resolve();
    if (!rebasing && at >= 0) document.previewTableCell(at, row, col, editor.serialize());
  }

  void commit() {
    final at = resolve();
    if (at >= 0 && editor.serialize() != _opening) document.setTableCell(at, row, col, editor.serialize());
  }

  bool reconcile() {
    final at = resolve();
    if (at < 0) return false;
    if (editor.serialize() == _opening) {
      final fresh = CellEditController(TableData.fromBlock(document.nodes[at].data).rows[row][col]);
      rebasing = true;
      // Set the value first: the setter shifts stale marks for its own diff,
      // then the correct remote marks replace them in the same snapshot.
      editor.value = fresh.value;
      editor.marks = fresh.marks;
      _opening = fresh.serialize();
      rebasing = false;
      fresh.dispose();
    } else {
      preview();
    }
    return true;
  }
}
