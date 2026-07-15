import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mica_flutter/editor/highlight.dart';
import 'package:mica_flutter/editor/markdown.dart';

// Three separate failures met here, all of them "the code block looks dead":
//
//  1. An unknown language makes the tokenizer bail out entirely, and pasted
//     fences say ```py / ```sh / ```yml far more often than the canonical name.
//     Every alias scored ZERO coloured characters.
//  2. YAML and CSS carry their meaning in `key:` / `prop:`, which a keyword set
//     cannot express — YAML coloured 3 characters out of 56.
//  3. ChatGPT labels Python as ```bash, and we honoured the label over our own
//     (correct) detection.

const base = TextStyle(fontSize: 12);

/// Characters the highlighter actually coloured — the only measure that
/// answers "does this block look highlighted".
int coloured(String code, String lang) {
  var n = 0;
  void walk(InlineSpan s) {
    if (s is TextSpan) {
      if (s.style?.color != null && s.text != null) n += s.text!.length;
      s.children?.forEach(walk);
    }
  }

  walk(buildCodeSpan(code, lang, base));
  return n;
}

/// The colour applied to [needle], or null if it was left plain.
Color? colourOf(String code, String lang, String needle) {
  Color? found;
  void walk(InlineSpan s) {
    if (s is TextSpan) {
      if (s.text == needle && found == null) found = s.style?.color;
      s.children?.forEach(walk);
    }
  }

  walk(buildCodeSpan(code, lang, base));
  return found;
}

void main() {
  group('language aliases', () {
    const py = 'def f(x):\n    return x + 1\n';
    const sh = '#!/bin/bash\necho hi\n';
    const yaml = 'name: mica\nversion: 1.2\n';

    test('py / python3 highlight exactly like python', () {
      final canonical = coloured(py, 'python');
      expect(canonical, greaterThan(0));
      expect(coloured(py, 'py'), canonical);
      expect(coloured(py, 'python3'), canonical);
    });

    test('sh / shell / zsh highlight exactly like bash', () {
      final canonical = coloured(sh, 'bash');
      expect(canonical, greaterThan(0));
      for (final a in ['sh', 'shell', 'zsh', 'console']) {
        expect(coloured(sh, a), canonical, reason: a);
      }
    });

    test('yml highlights exactly like yaml', () {
      expect(coloured(yaml, 'yml'), coloured(yaml, 'yaml'));
    });

    test('the usual suspects all resolve', () {
      expect(canonicalCodeLanguage('JS'), 'javascript');
      expect(canonicalCodeLanguage('TSX'), 'typescript');
      expect(canonicalCodeLanguage('rs'), 'rust');
      expect(canonicalCodeLanguage('golang'), 'go');
      expect(canonicalCodeLanguage('c++'), 'cpp');
      expect(canonicalCodeLanguage(' Text '), 'plaintext');
    });

    test('an unknown language is passed through, not mangled', () {
      expect(canonicalCodeLanguage('brainfuck'), 'brainfuck');
      expect(coloured('++++.', 'brainfuck'), 0, reason: 'plain, but no crash');
    });

    test('resolveCodeLanguage canonicalises a chosen alias', () {
      expect(resolveCodeLanguage('x = 1', 'py'), 'python');
      expect(resolveCodeLanguage('def f():\n  pass', 'auto'), 'python');
    });
  });

  group('structural highlighting', () {
    test('a YAML key is coloured, and differently from its value', () {
      const yaml = 'name: mica\nversion: 1.2\nservices:\n  api:\n    image: foo\n';
      expect(coloured(yaml, 'yaml'), greaterThan(20),
          reason: 'this used to colour 3 characters out of 56');
      final key = colourOf(yaml, 'yaml', 'name');
      expect(key, isNotNull, reason: 'the key must not be plain text');
      expect(colourOf(yaml, 'yaml', 'api'), key, reason: 'nested keys too');
    });

    test('a colon inside a YAML value is not mistaken for a key', () {
      const yaml = 'url: http://example.com\n';
      expect(colourOf(yaml, 'yaml', 'url'), isNotNull);
      expect(colourOf(yaml, 'yaml', 'http'), isNull,
          reason: 'only a line-leading token can be a key');
    });

    test('a YAML list item key is still a key', () {
      const yaml = 'items:\n  - name: a\n  - name: b\n';
      expect(colourOf(yaml, 'yaml', 'items'), isNotNull);
      expect(colourOf(yaml, 'yaml', 'name'), isNotNull,
          reason: 'the `- ` marker must not disqualify the key after it');
    });

    test('a YAML comment still wins over the key rule', () {
      const yaml = '# note: not a key\nreal: 1\n';
      expect(colourOf(yaml, 'yaml', '# note: not a key'), isNotNull);
    });

    test('a CSS property and selector are coloured differently', () {
      const css = 'body {\n  color: red;\n  margin: 0;\n}\n';
      expect(coloured(css, 'css'), greaterThan(10),
          reason: 'this used to colour 1 character out of 36');
      final prop = colourOf(css, 'css', 'color');
      expect(prop, isNotNull);
      expect(colourOf(css, 'css', 'body'), isNot(prop),
          reason: 'a selector is not a property');
    });

    test('an HTML tag, attribute and comment each get colour', () {
      const html = '<!-- hi -->\n<div class="a" id="b">text</div>\n';
      expect(colourOf(html, 'html', '<div'), isNotNull);
      expect(colourOf(html, 'html', 'class'), isNotNull);
      expect(colourOf(html, 'html', '<!-- hi -->'), isNotNull,
          reason: 'HTML comments are neither // nor /* */');
    });

    test('a JSON key is distinguishable from a string value', () {
      const json = '{"a": "x"}';
      final key = colourOf(json, 'json', '"a"');
      final value = colourOf(json, 'json', '"x"');
      expect(key, isNotNull);
      expect(value, isNotNull);
      expect(key, isNot(value),
          reason: 'both were green — the structure read as one blob');
    });
  });

  group('a pasted fence that lies about its language', () {
    // The reported case, verbatim in shape: ChatGPT hands over Python inside a
    // ```bash fence.
    const pythonInBash = '''
```bash
def process_data(file_path):
    results = []
    with open(file_path, 'r') as file:
        for line in file:
            if float(line.strip()) > 10.0:
                results.append(line)
    return results
```
''';

    test('python labelled bash is retagged python', () {
      expect(retagMislabeledFences(pythonInBash), contains('```python'));
      expect(retagMislabeledFences(pythonInBash), isNot(contains('```bash')));
    });

    test('a correct label is left exactly alone', () {
      const bash = '```bash\n#!/bin/bash\necho hi\n```\n';
      expect(retagMislabeledFences(bash), bash);
      const py = '```python\ndef f():\n    pass\n```\n';
      expect(retagMislabeledFences(py), py);
    });

    test('an alias that is right is not rewritten to its canonical name', () {
      // ```py IS python — retagging would be churn, not a correction.
      const py = '```py\ndef f():\n    pass\n```\n';
      expect(retagMislabeledFences(py), py);
    });

    test('a language we cannot identify is left alone', () {
      const ruby = '```ruby\nputs "hi"\n```\n';
      expect(retagMislabeledFences(ruby), ruby);
      const unlabelled = '```\ndef f():\n    pass\n```\n';
      expect(retagMislabeledFences(unlabelled), unlabelled,
          reason: 'no label to correct — auto-detect handles this already');
    });

    test('a shebang outranks the label', () {
      const shInPython = '```python\n#!/bin/bash\necho hi\n```\n';
      expect(retagMislabeledFences(shInPython), contains('```bash'));
    });

    test('prose between blocks is untouched, and every block is seen', () {
      const doc = '''
before

```bash
def a():
    pass
```

middle

```js
const x = 1;
```

after
''';
      final out = retagMislabeledFences(doc);
      expect(out, contains('```python'));
      expect(out, contains('```js'), reason: 'js is not contradicted');
      expect(out, contains('before'));
      expect(out, contains('middle'));
      expect(out, contains('after'));
    });

    test('a fence-looking line INSIDE a block is not treated as a fence', () {
      const doc = '```bash\ndef a():\n    s = "```python"\n```\n';
      final out = retagMislabeledFences(doc);
      expect(out, contains('s = "```python"'), reason: 'body is never rewritten');
    });
  });

  group('strongLanguageSignature refuses to guess', () {
    test('it stays silent on what it cannot pin down', () {
      // `#include` is C AND C++; `public class` is Java, C# and Kotlin. A
      // signature that cannot name ONE language must return null, or it would
      // retag correct code into the wrong language.
      expect(strongLanguageSignature('#include <stdio.h>\nint main(){}'), isNull);
      expect(
        strongLanguageSignature('public class Foo { void a() {} }'),
        isNull,
      );
      expect(strongLanguageSignature('echo hello'), isNull,
          reason: 'a bare word is never strong evidence');
      expect(strongLanguageSignature('print(1)'), isNull,
          reason: 'print( exists in half the languages alive');
      expect(strongLanguageSignature(''), isNull);
    });

    test('it does fire on real structure', () {
      expect(strongLanguageSignature('def f(x):\n  return x'), 'python');
      expect(
        strongLanguageSignature('fn main() {\n  let mut x = 1;\n}'),
        'rust',
      );
      expect(
        strongLanguageSignature('package main\n\nfunc main() {\n}'),
        'go',
      );
    });

    test('a diff of python is not python', () {
      // The +/- gutter breaks the line-anchored signature, which is the point:
      // a diff is a diff.
      expect(strongLanguageSignature('+def f(x):\n-    return x'), isNull);
    });
  });
}
