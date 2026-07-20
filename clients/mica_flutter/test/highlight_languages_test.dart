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
  // AppFlowy ships a code-block picker with no HTML and no C#. It intersects a
  // hardcoded name list against its highlighter's grammar ids, and entries whose
  // spelling differs (`HTML` vs hljs's `xml`, `C#` vs `csharp`) are dropped
  // silently — nothing checks the intersection, so the gap shipped.
  //
  // Our list (`kCodeLanguages`) and our tokenizer configs are likewise two
  // hand-maintained places. This group keeps them in step: an entry that reaches
  // the dropdown but colours nothing is worse than an absent one, because the
  // user picks it and concludes the highlighter is broken.
  group('every language offered in the picker actually works', () {
    // One representative line per language. Deliberately boring — the question
    // is "does this entry produce ANY colour", not tokenizer quality.
    const samples = <String, String>{
      'dart': 'void main() { return; }',
      'javascript': 'const x = 1;',
      'typescript': 'let x: number = 1;',
      'python': 'def f():\n    return 1\n',
      'rust': 'fn main() { let x = 1; }',
      'go': 'func main() { return }',
      'java': 'public class A { }',
      'c': 'int main() { return 0; }',
      'cpp': 'int main() { return 0; }',
      'json': '{"a": 1}',
      'yaml': 'name: mica\n',
      'sql': 'SELECT * FROM t;',
      'bash': 'echo hi\n',
      'powershell': r'if ($true) { exit 0 }',
      'html': '<div class="a">x</div>',
      'css': 'body { color: red; }',
      'mermaid': 'graph TD\n  A --> B\n',
      'kotlin': 'fun main() { val x = 1 }',
      'swift': 'func f() -> Int { return 1 }',
      'csharp': 'public class A { void B() { } }',
      'php': '<?php echo "hi";',
      'ruby': 'def f\n  puts "hi"\nend\n',
      'objective-c': '@interface Foo : NSObject\n@end\n',
      'lua': 'local x = 1\n',
      'perl': 'my \$x = 1;\n',
      'r': 'f <- function(x) { x + 1 }\n',
      'scala': 'object A { def b = 1 }',
      'groovy': 'def x = [1, 2]\n',
      'elixir': 'defmodule A do\n  def b, do: :ok\nend\n',
      'haskell': 'main :: IO ()\nmain = putStrLn "hi"\n',
      'zig': 'const std = @import("std");\n',
      'dockerfile': 'FROM alpine:3.19\nRUN apk add curl\n',
      'xml': '<?xml version="1.0"?>\n<a b="c"/>\n',
      'toml': '[package]\nname = "mica"\n',
      'ini': '[core]\nkey = value\n',
      'diff': '--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n',
      'markdown': '# Title\n\nSome **bold** text.\n',
      'graphql': 'query { user(id: 1) { name } }',
      'protobuf': 'message A {\n  string b = 1;\n}\n',
      'nginx': 'server {\n    listen 80;\n}\n',
      'makefile': 'build:\n\tgo build ./...\n',
      'latex': r'\documentclass{article}' '\n' r'\begin{document}' '\nHi\n',
      'nix': 'let x = 1; in x\n',
    };

    test('every entry has a sample here', () {
      // Forces this test to be updated when a language is added, instead of the
      // new entry quietly escaping coverage.
      final expected =
          kCodeLanguages.where((l) => l != 'auto' && l != 'plaintext');
      expect(samples.keys.toSet(), expected.toSet());
    });

    test('every entry colours something', () {
      for (final entry in samples.entries) {
        expect(
          coloured(entry.value, entry.key),
          greaterThan(0),
          reason: '${entry.key} is in the dropdown but highlights nothing',
        );
      }
    });

    test('every entry is its own canonical name, not an alias', () {
      // An entry that canonicalises to something else would highlight as that
      // other language while claiming to be itself.
      for (final lang in kCodeLanguages.where((l) => l != 'auto')) {
        expect(canonicalCodeLanguage(lang), lang, reason: lang);
      }
    });
  });

  group('powershell', () {
    test('it is offered, and highlights', () {
      expect(kCodeLanguages, contains('powershell'));
      expect(
          coloured(r'if ($x -eq 1) { exit 0 }', 'powershell'), greaterThan(0));
    });

    test('ps / ps1 / pwsh resolve to it', () {
      for (final a in ['ps', 'ps1', 'pwsh', 'posh', 'PS1']) {
        expect(canonicalCodeLanguage(a), 'powershell', reason: a);
      }
    });

    test('a backtick is an escape, not a string delimiter', () {
      // The generic default treats ` as a string quote. In PowerShell it escapes
      // the next character, so the default would open a string at the first `n
      // and swallow the rest of the line as one green blob.
      //
      // The control has the SAME LENGTH — a backtick swapped for an ordinary
      // letter. Comparing against `"anb"` instead would fail on the missing
      // character alone and prove nothing about delimiters.
      expect(
        coloured(r'Write-Output "a`nb"', 'powershell'),
        coloured(r'Write-Output "axnb"', 'powershell'),
        reason: 'a backtick must tokenize as an ordinary character in a string',
      );
    });

    test('keywords are case-insensitive', () {
      // Real scripts mix ForEach / foreach / FOREACH freely.
      final lower = coloured(r'foreach ($i in $a) { }', 'powershell');
      expect(lower, greaterThan(0));
      expect(coloured(r'ForEach ($i in $a) { }', 'powershell'), lower);
      expect(coloured(r'FOREACH ($i in $a) { }', 'powershell'), lower);
    });

    test('a # comment is a comment', () {
      expect(colourOf('# note\nexit 0\n', 'powershell', '# note'), isNotNull);
    });
  });

  // The default `strings` is `['"', "'", '`']`, the default `lineComments` is
  // `['//']` and `blockComments` defaults to `/* */`. For most of the languages
  // added alongside PowerShell at least one of those defaults is actively
  // WRONG, and the failure mode is the loud one: a delimiter that never closes
  // paints the rest of the line — or the paragraph — a single colour. "It
  // colours something" cannot catch that, so each decision gets a test.
  group('tokenizer defaults that would be wrong', () {
    /// Longest run of consecutive characters carrying one colour. A swallowed
    /// line shows up here and nowhere else.
    int longestRun(String code, String lang) {
      var best = 0;
      void walk(InlineSpan s) {
        if (s is TextSpan) {
          if (s.style?.color != null && s.text != null && s.text!.length > best) {
            best = s.text!.length;
          }
          s.children?.forEach(walk);
        }
      }

      walk(buildCodeSpan(code, lang, base));
      return best;
    }

    test('an apostrophe in markdown prose is not a string', () {
      // "don't" would open a string that never closes.
      const md = "It doesn't matter, and it isn't a string either.\n";
      expect(longestRun(md, 'markdown'), lessThan(10),
          reason: 'an unclosed quote would paint the whole sentence');
    });

    test('a backtick in markdown opens inline code, not a string', () {
      expect(colourOf('use `git log` here\n', 'markdown', '`git log`'),
          isNotNull);
    });

    test('a markdown heading and a bullet are coloured', () {
      expect(colourOf('# Title\n', 'markdown', '# Title'), isNotNull);
      expect(colourOf('- item\n', 'markdown', '-'), isNotNull);
    });

    test('an apostrophe in Haskell is an identifier character', () {
      // `foldl'` and `x'` are ordinary names; as a quote they would eat the
      // rest of the line. Control has the same length with a plain letter.
      expect(longestRun("let xs' = foldl' f z ys\n", 'haskell'),
          longestRun('let xsA = foldlA f z ys\n', 'haskell'));
    });

    test('a backtick in Ruby is a subprocess, not a string', () {
      expect(longestRun('x = `ls -la`\ny = 1\n', 'ruby'), lessThan(8));
    });

    test('a Scala symbol literal does not open a string', () {
      // The rest of the line has to be long enough to be worth swallowing —
      // the scanner gives up at a newline, so a short line would pass either
      // way and prove nothing.
      expect(longestRun("val sym = 'notAString and more text here\n", 'scala'),
          lessThan(8));
    });

    test('diff colours added and removed lines differently', () {
      const d = '--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-old\n+new\n';
      final added = colourOf(d, 'diff', '+new');
      final removed = colourOf(d, 'diff', '-old');
      expect(added, isNotNull);
      expect(removed, isNotNull);
      expect(added, isNot(removed), reason: 'a diff is +/- or it is nothing');
      expect(colourOf(d, 'diff', '+++ b/x'), isNot(added),
          reason: 'the file header is not an added line');
    });

    test('a quote inside a diff body does not pair across lines', () {
      const d = '-  say "hi\n+  say "hello"\n';
      expect(colourOf(d, 'diff', '-  say "hi'), isNotNull,
          reason: 'the whole line is one token; quotes are just characters');
    });

    test('an INI comment can start with ; as well as #', () {
      expect(colourOf('; note\nk = 1\n', 'ini', '; note'), isNotNull);
      expect(colourOf('# note\nk = 1\n', 'ini', '# note'), isNotNull);
    });

    test('a Lua -- comment is a comment, and -- blocks are not /* */', () {
      expect(colourOf('-- note\nlocal x = 1\n', 'lua', '-- note'), isNotNull);
    });

    test('a LaTeX % is a comment and \\cmd is a keyword', () {
      expect(colourOf('% note\n', 'latex', '% note'), isNotNull);
      expect(colourOf(r'\textbf{hi}', 'latex', r'\textbf'), isNotNull);
    });

    test('Dockerfile instructions are case-insensitive', () {
      final upper = coloured('FROM alpine\nRUN true\n', 'dockerfile');
      expect(upper, greaterThan(0));
      expect(coloured('from alpine\nrun true\n', 'dockerfile'), upper);
    });

    test('a TOML table and key are coloured differently', () {
      const toml = '[package]\nname = "mica"\n';
      final table = colourOf(toml, 'toml', '[package]');
      expect(table, isNotNull);
      expect(colourOf(toml, 'toml', 'name'), isNot(table));
    });

    test('an nginx directive is coloured at the head of a line only', () {
      const conf = 'server {\n    listen 80;\n}\n';
      expect(colourOf(conf, 'nginx', 'listen'), isNotNull);
      expect(colourOf(conf, 'nginx', 'server'), isNotNull);
    });

    test('a Makefile target is not confused with a := assignment', () {
      const mk = 'CC := gcc\nbuild:\n\t\$(CC) main.c\n';
      final target = colourOf(mk, 'makefile', 'build');
      expect(target, isNotNull);
      expect(colourOf(mk, 'makefile', 'CC'), isNot(target),
          reason: 'a variable is not a target');
    });

    test('XML gets its own entry, and CDATA is not markup', () {
      expect(canonicalCodeLanguage('xml'), 'xml');
      expect(canonicalCodeLanguage('svg'), 'xml');
      const xml = '<a><![CDATA[<not a tag>]]></a>';
      expect(colourOf(xml, 'xml', '<![CDATA[<not a tag>]]>'), isNotNull);
    });

    test('Zig has no block comments', () {
      // `/*` is division-then-star in Zig; treating it as a comment opener
      // would grey out everything after it.
      expect(longestRun('const a = b / *c;\nconst d = 1;\n', 'zig'),
          lessThan(10));
    });
  });

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

  group('auto-detection covers what people actually paste', () {
    // YAML was missing outright, so pasting a config into an `auto` block
    // resolved to `plaintext` — the "just set it to auto" answer didn't even
    // work. CSS was missing too.
    const compose =
        'services:\n  api:\n    image: mica\n    ports:\n      - "8080:80"\n';

    test('yaml is detected at all', () {
      expect(detectLanguage(compose), 'yaml');
      expect(
        detectLanguage('name: CI\non:\n  push:\n    tags: ["v*"]\n'),
        'yaml',
      );
      expect(detectLanguage('---\ntitle: hi\ndate: 2026-01-01\n---\n'), 'yaml',
          reason: 'a document marker is enough on its own');
    });

    test('an auto block follows pasted yaml', () {
      // The scenario as reported: this is what `auto` is for, and it returned
      // plaintext before.
      expect(resolveCodeLanguage(compose, 'auto'), 'yaml');
      expect(resolveCodeLanguage(compose, null), 'yaml');
    });

    test('a pinned block does NOT follow pasted yaml', () {
      // Deliberately: the author chose python. A choice that the next edit
      // silently overturns is not a choice.
      expect(resolveCodeLanguage(compose, 'python'), 'python');
    });

    test('yaml holding a python one-liner is still yaml', () {
      // `command: python -c "print(1)"` trips detectLanguage's weak `print(`
      // rule — hence YAML sits above it, and Python's real signature (`def
      // f():`) wins earlier via strongLanguageSignature regardless.
      expect(
        detectLanguage('services:\n  x:\n    command: python -c "print(1)"\n'),
        'yaml',
      );
    });

    test('css is detected, and not mistaken for yaml', () {
      // `color: red;` inside braces looks exactly like a YAML key.
      expect(detectLanguage('body {\n  color: red;\n  margin: 0;\n}\n'), 'css');
      expect(
        detectLanguage('@media (max-width: 600px) {\n  .a { display: none; }\n}\n'),
        'css',
      );
    });

    test('yaml does not steal from its neighbours', () {
      expect(detectLanguage('{\n  "a": 1,\n  "b": "x"\n}'), 'json');
      expect(detectLanguage('def f(x):\n    return x\n'), 'python');
      expect(detectLanguage('#!/bin/bash\necho hi\n'), 'bash');
      expect(detectLanguage('package main\n\nfunc main() {\n}\n'), 'go');
      expect(detectLanguage('just some words\nand more words\n'), 'plaintext');
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
