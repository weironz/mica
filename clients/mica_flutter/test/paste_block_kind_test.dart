import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/editor.dart';
import 'package:mica_flutter/editor/html_to_markdown.dart';
import 'package:mica_flutter/editor/model.dart';

/// Regression: copying an H2 and pasting it into a sentence left a SECOND H2
/// block under that sentence — the paste overruled a block kind the user had
/// already chosen for that line. The rule (AppFlowy `pasteSingleLineNode`,
/// AFFiNE `canMerge`, ProseMirror#231): a pasted block merges into the caret's
/// line as inline text UNLESS that line ends up empty, in which case it has no
/// kind worth defending and adopts the pasted one.
void main() {
  // Guards the branch's own precondition: the copy → clipboard → paste round
  // trip has to yield a SINGLE LINE, or `_handleRichPaste` never reaches the
  // inline test and this whole fix is dead code.
  group('copying one block yields single-line markdown', () {
    EditorController withOne(EditorNode n) {
      final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
      c.load([n]);
      c.selection = DocSelection(
        anchor: const DocPosition(0, 0),
        focus: DocPosition(0, n.text.length),
      );
      return c;
    }

    final cases = {
      'heading':
          EditorNode(id: 'h', kind: 'heading', text: 'Heading', data: {'level': 2}),
      'bulleted_list': EditorNode(id: 'b', kind: 'bulleted_list', text: 'item'),
      'quote': EditorNode(id: 'q', kind: 'quote', text: 'quoted'),
    };
    cases.forEach((kind, node) {
      test(kind, () {
        final md =
            htmlToMarkdown(withOne(node).selectionHtml(imageUrls: const {}));
        expect(md.contains('\n'), isFalse,
            reason: 'must stay on one line: "$md"');
        expect(markdownToBlocks(md).single.kind, kind);
      });
    });
  });

  group('pasteMergesInline', () {
    // The reported bug: caret parked in a sentence, paste an H2.
    test('caret at the end of body text → merge (was: a second H2 block)', () {
      expect(
        pasteMergesInline(
          specKind: 'heading',
          targetKind: 'paragraph',
          targetLength: 9,
          selFrom: 9,
          selTo: 9,
        ),
        isTrue,
      );
    });

    test('caret in the middle of body text → merge', () {
      expect(
        pasteMergesInline(
          specKind: 'heading',
          targetKind: 'paragraph',
          targetLength: 9,
          selFrom: 4,
          selTo: 4,
        ),
        isTrue,
      );
    });

    test('part of the line selected → merge (text survives)', () {
      expect(
        pasteMergesInline(
          specKind: 'heading',
          targetKind: 'paragraph',
          targetLength: 9,
          selFrom: 0,
          selTo: 4,
        ),
        isTrue,
      );
    });

    // The other half of the rule — these must KEEP the pasted heading.
    test('blank line → block path, so the heading survives', () {
      expect(
        pasteMergesInline(
          specKind: 'heading',
          targetKind: 'paragraph',
          targetLength: 0,
          selFrom: 0,
          selTo: 0,
        ),
        isFalse,
      );
    });

    test('whole line selected → block path, so the heading survives', () {
      expect(
        pasteMergesInline(
          specKind: 'heading',
          targetKind: 'paragraph',
          targetLength: 9,
          selFrom: 0,
          selTo: 9,
        ),
        isFalse,
      );
    });

    // An empty NON-default block is a kind the user set on purpose: it keeps
    // its own and swallows the pasted text (AppFlowy limits its exemption to
    // `type == ParagraphBlockKeys.type` for exactly this reason).
    test('emptied heading keeps its own level, takes the text inline', () {
      expect(
        pasteMergesInline(
          specKind: 'heading',
          targetKind: 'heading',
          targetLength: 0,
          selFrom: 0,
          selTo: 0,
        ),
        isTrue,
      );
    });

    test('emptied quote keeps being a quote', () {
      expect(
        pasteMergesInline(
          specKind: 'bulleted_list',
          targetKind: 'quote',
          targetLength: 6,
          selFrom: 0,
          selTo: 6,
        ),
        isTrue,
      );
    });

    // A plain paragraph imposes no kind, so it merges everywhere — this is the
    // pre-existing behaviour the fix must not disturb.
    test('a paragraph spec merges even onto a blank line', () {
      expect(
        pasteMergesInline(
          specKind: 'paragraph',
          targetKind: 'paragraph',
          targetLength: 0,
          selFrom: 0,
          selTo: 0,
        ),
        isTrue,
      );
    });

    // Kinds with no inline form must never be flattened into a text run.
    for (final kind in const [
      'code_block',
      'table',
      'image',
      'divider',
      'math_block',
      'footnote_def',
    ]) {
      test('$kind never merges inline', () {
        expect(
          pasteMergesInline(
            specKind: kind,
            targetKind: 'paragraph',
            targetLength: 9,
            selFrom: 9,
            selTo: 9,
          ),
          isFalse,
        );
      });
    }
  });

  group('EditorNode.isInlineTextKind', () {
    test('text-flow kinds', () {
      for (final k in const [
        'paragraph',
        'heading',
        'quote',
        'bulleted_list',
        'numbered_list',
        'todo',
      ]) {
        expect(EditorNode.isInlineTextKind(k), isTrue, reason: k);
      }
    });
    test('everything else', () {
      for (final k in const [
        'code_block',
        'table',
        'image',
        'divider',
        'math_block',
        'footnote_def',
      ]) {
        expect(EditorNode.isInlineTextKind(k), isFalse, reason: k);
      }
    });
  });
}
