import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/controller.dart';
import 'package:mica_flutter/editor/model.dart';

// Copying an `auto` code block out of Mica emitted a bare ``` — Typora, GitHub
// and VS Code then had a block with no language and no way to work one out,
// because `auto` is a Mica concept and doesn't exist on their side. The
// clipboard is interchange, so it hands over the resolved answer instead.
//
// Export (crates/markdown) deliberately does NOT do this: it is document
// serialization, round-trip is an invariant there, and a bare fence the author
// typed must come back out bare. See EditorController._copyLanguage.

const yaml = 'services:\n  api:\n    image: mica\n    ports:\n      - "80:80"\n';
const python = 'def f(x):\n    return x + 1\n';

void main() {
  EditorController withCode(Map<String, dynamic> data, String text) {
    final c = EditorController(rootBlockId: 'root', onOps: (_) async {});
    c.load([
      EditorNode(id: 'a', kind: 'paragraph', text: 'before'),
      EditorNode(id: 'c', kind: 'code_block', text: text, data: data),
      EditorNode(id: 'b', kind: 'paragraph', text: 'after'),
    ]);
    // Select the whole document, the way Ctrl+A then Ctrl+C does.
    c.setSelection(DocSelection(
      anchor: const DocPosition(0, 0),
      focus: DocPosition(2, c.nodes[2].text.length),
    ));
    return c;
  }

  group('markdown on the clipboard', () {
    test('an auto block carries the language it detected', () {
      final md = withCode({}, yaml).selectionText();
      expect(md, contains('```yaml'),
          reason: 'this used to be a bare ``` and Typora showed no language');
    });

    test('a pinned block carries what the author pinned', () {
      final md = withCode({'language': 'python'}, python).selectionText();
      expect(md, contains('```python'));
    });

    test('a pinned alias goes out verbatim, not canonicalised', () {
      // ```py IS python. Rewriting the author's own word is churn, not a
      // correction — the same call retagMislabeledFences makes on paste.
      final md = withCode({'language': 'py'}, python).selectionText();
      expect(md, contains('```py\n'));
      expect(md, isNot(contains('```python')));
    });

    test('undetectable content stays bare — no invented label', () {
      // 'plaintext' is detection saying "no idea". Stamping it on would dress a
      // guess up as a decision.
      final md = withCode({}, 'just some words\nand more words\n')
          .selectionText();
      expect(md, contains('```\n'));
      expect(md, isNot(contains('```plaintext')));
    });

    test('a block pinned to plaintext says plaintext', () {
      // Bare is for "we could not tell". Pinned plaintext is a decision — the
      // author said this is not code, and that survives.
      final md = withCode({'language': 'plaintext'}, python).selectionText();
      expect(md, contains('```plaintext'));
    });
  });

  group('html on the clipboard', () {
    // Typora and Word prefer text/html when the clipboard offers both, so the
    // class has to carry the language too or the fix only half works.
    test('an auto block gets a language- class', () {
      final html = withCode({}, yaml).selectionHtml();
      expect(html, contains('class="language-yaml"'));
    });

    test('a pinned block gets its own class', () {
      final html = withCode({'language': 'python'}, python).selectionHtml();
      expect(html, contains('class="language-python"'));
    });

    test('undetectable content gets no class at all', () {
      final html =
          withCode({}, 'just some words\nand more words\n').selectionHtml();
      expect(html, contains('<pre><code>'));
      expect(html, isNot(contains('language-plaintext')));
    });
  });
}
