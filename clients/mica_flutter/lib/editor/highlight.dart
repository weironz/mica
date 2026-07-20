import 'package:flutter/widgets.dart';

/// In-house syntax highlighter for code blocks. A single generic tokenizer
/// (comments, strings, numbers, identifiers, keywords) is tuned per language by
/// a small [_Lang] config — no third-party highlighter dependency. Language can
/// be auto-detected from the content or chosen explicitly.

/// Languages offered in the code-block dropdown. `auto` detects from content.
const List<String> kCodeLanguages = [
  'auto',
  'plaintext',
  'dart',
  'javascript',
  'typescript',
  'python',
  'rust',
  'go',
  'java',
  'c',
  'cpp',
  'json',
  'yaml',
  'sql',
  'bash',
  'powershell',
  'html',
  'css',
  'mermaid',
];

// Light-theme token palette.
const Color _kwColor = Color(0xFF7C3AED); // keyword — purple
const Color _strColor = Color(0xFF0A7D34); // string — green
const Color _comColor = Color(0xFF6B7280); // comment — grey
const Color _numColor = Color(0xFFB45309); // number — amber
const Color _fnColor = Color(0xFF2563EB); // function call — blue
const Color _keyColor = Color(0xFF0F766E); // key / property / tag — teal

/// Fence labels people actually write, mapped to the name the tokenizer knows.
///
/// Not cosmetic: an unknown language makes [buildCodeSpan] bail out and render
/// the block as flat grey text, and pasted fences say ```py / ```sh / ```yml far
/// more often than the canonical spelling. Every alias here scored **zero**
/// coloured characters before the map existed.
const Map<String, String> _languageAliases = {
  'py': 'python',
  'python3': 'python',
  'sh': 'bash',
  'shell': 'bash',
  'zsh': 'bash',
  'ksh': 'bash',
  'console': 'bash',
  'shell-session': 'bash',
  'yml': 'yaml',
  // `ps1` is the file extension people paste from; `ps` and `pwsh` are what
  // shiki/vscode accept, so pasted fences use all three.
  'ps': 'powershell',
  'ps1': 'powershell',
  'pwsh': 'powershell',
  'posh': 'powershell',
  'rs': 'rust',
  'js': 'javascript',
  'jsx': 'javascript',
  'mjs': 'javascript',
  'cjs': 'javascript',
  'node': 'javascript',
  'ts': 'typescript',
  'tsx': 'typescript',
  'golang': 'go',
  'c++': 'cpp',
  'cxx': 'cpp',
  'cc': 'cpp',
  'hpp': 'cpp',
  'h': 'c',
  'htm': 'html',
  'xml': 'html',
  'svg': 'html',
  'xhtml': 'html',
  'jsonc': 'json',
  'json5': 'json',
  'sqlite': 'sql',
  'psql': 'sql',
  'postgres': 'sql',
  'postgresql': 'sql',
  'mysql': 'sql',
  'text': 'plaintext',
  'txt': 'plaintext',
  'none': 'plaintext',
  'plain': 'plaintext',
};

/// The canonical name for a fence label / dropdown choice: `py` → `python`.
/// Unknown names pass through unchanged (and simply won't highlight).
String canonicalCodeLanguage(String name) {
  final n = name.trim().toLowerCase();
  return _languageAliases[n] ?? n;
}

/// A structural pattern — syntax that carries a language's meaning without
/// being a *word*, so the keyword set is powerless to express it.
///
/// This is the whole reason YAML looked dead: its salience is `key:`, and a
/// keyword list can only match `true`/`false`/`null`, which barely occur. Same
/// for a CSS property, an HTML tag, a JSON key.
///
/// [source] is anchored at the scan position. Group 1 is coloured when present,
/// otherwise the whole match; either way it must start at the match's start, so
/// the scanner can simply skip past what it coloured.
class _Rule {
  _Rule(String source, this.color, {this.leadOnly = false})
      : pattern = RegExp(source);

  final RegExp pattern;
  final Color color;

  /// Only try at a line's first real token — after indentation and after
  /// YAML's `- ` item marker. This is what separates a `key:` from a colon
  /// that merely appears inside a value (`url: http://x`).
  final bool leadOnly;
}

class _Lang {
  const _Lang({
    this.keywords = const {},
    this.lineComments = const ['//'],
    this.blockComments = true,
    this.strings = const ['"', "'", '`'],
    this.caseInsensitive = false,
    this.rules = const [],
  });

  final Set<String> keywords;
  final List<String> lineComments;
  final bool blockComments;
  final List<String> strings;
  final bool caseInsensitive;

  /// Tried before comments/strings/keywords, in order. See [_Rule].
  final List<_Rule> rules;
}

const _hashLike = ['#'];

final Map<String, _Lang> _langs = {
  'dart': const _Lang(
    keywords: {
      'abstract', 'as', 'async', 'await', 'break', 'case', 'catch', 'class',
      'const', 'continue', 'default', 'do', 'dynamic', 'else', 'enum', 'export',
      'extends', 'extension', 'external', 'factory', 'false', 'final', 'finally',
      'for', 'get', 'if', 'implements', 'import', 'in', 'is', 'late', 'library',
      'mixin', 'new', 'null', 'on', 'operator', 'part', 'required', 'rethrow',
      'return', 'set', 'static', 'super', 'switch', 'this', 'throw', 'true',
      'try', 'typedef', 'var', 'void', 'while', 'with', 'yield',
    },
  ),
  'javascript': const _Lang(
    keywords: {
      'const', 'let', 'var', 'function', 'return', 'if', 'else', 'for', 'while',
      'do', 'switch', 'case', 'break', 'continue', 'class', 'extends', 'super',
      'new', 'this', 'typeof', 'instanceof', 'in', 'of', 'try', 'catch',
      'finally', 'throw', 'async', 'await', 'yield', 'import', 'export', 'from',
      'default', 'null', 'undefined', 'true', 'false', 'void', 'delete',
    },
  ),
  'typescript': const _Lang(
    keywords: {
      'const', 'let', 'var', 'function', 'return', 'if', 'else', 'for', 'while',
      'do', 'switch', 'case', 'break', 'continue', 'class', 'extends', 'super',
      'new', 'this', 'typeof', 'instanceof', 'in', 'of', 'try', 'catch',
      'finally', 'throw', 'async', 'await', 'yield', 'import', 'export', 'from',
      'default', 'null', 'undefined', 'true', 'false', 'void', 'delete',
      'interface', 'type', 'enum', 'implements', 'public', 'private',
      'protected', 'readonly', 'as', 'namespace', 'declare', 'keyof',
    },
  ),
  'python': const _Lang(
    keywords: {
      'def', 'return', 'if', 'elif', 'else', 'for', 'while', 'break', 'continue',
      'class', 'import', 'from', 'as', 'try', 'except', 'finally', 'raise',
      'with', 'lambda', 'yield', 'global', 'nonlocal', 'pass', 'True', 'False',
      'None', 'and', 'or', 'not', 'in', 'is', 'async', 'await', 'del', 'assert',
    },
    lineComments: _hashLike,
    blockComments: false,
  ),
  'rust': const _Lang(
    keywords: {
      'fn', 'let', 'mut', 'const', 'static', 'if', 'else', 'match', 'for',
      'while', 'loop', 'break', 'continue', 'return', 'struct', 'enum', 'impl',
      'trait', 'pub', 'use', 'mod', 'crate', 'self', 'super', 'as', 'where',
      'move', 'ref', 'dyn', 'async', 'await', 'unsafe', 'extern', 'type',
      'true', 'false', 'Some', 'None', 'Ok', 'Err',
    },
  ),
  'go': const _Lang(
    keywords: {
      'func', 'var', 'const', 'package', 'import', 'if', 'else', 'for', 'range',
      'switch', 'case', 'default', 'break', 'continue', 'return', 'type',
      'struct', 'interface', 'map', 'chan', 'go', 'defer', 'select',
      'fallthrough', 'nil', 'true', 'false',
    },
  ),
  'java': const _Lang(
    keywords: {
      'int', 'long', 'short', 'char', 'float', 'double', 'boolean', 'void',
      'class', 'public', 'private', 'protected', 'static', 'final', 'return',
      'if', 'else', 'for', 'while', 'do', 'switch', 'case', 'break', 'continue',
      'new', 'this', 'super', 'extends', 'implements', 'interface', 'enum',
      'try', 'catch', 'finally', 'throw', 'throws', 'import', 'package', 'true',
      'false', 'null', 'abstract', 'instanceof',
    },
  ),
  'c': const _Lang(
    keywords: {
      'int', 'long', 'short', 'char', 'float', 'double', 'void', 'struct',
      'union', 'enum', 'static', 'const', 'return', 'if', 'else', 'for', 'while',
      'do', 'switch', 'case', 'break', 'continue', 'sizeof', 'typedef',
      'unsigned', 'signed', 'extern', 'volatile', 'register', 'goto', 'NULL',
    },
  ),
  'cpp': const _Lang(
    keywords: {
      'int', 'long', 'short', 'char', 'float', 'double', 'bool', 'void',
      'class', 'struct', 'public', 'private', 'protected', 'static', 'const',
      'return', 'if', 'else', 'for', 'while', 'do', 'switch', 'case', 'break',
      'continue', 'new', 'delete', 'this', 'template', 'typename', 'namespace',
      'using', 'auto', 'true', 'false', 'nullptr', 'virtual', 'override',
      'sizeof', 'enum', 'try', 'catch', 'throw',
    },
  ),
  'json': _Lang(
    keywords: const {'true', 'false', 'null'},
    lineComments: const [],
    blockComments: false,
    strings: const ['"'],
    rules: [
      // A key is a string too, so without this every key and every value came
      // out the same green and the structure read as one undifferentiated blob.
      _Rule(r'"(?:[^"\\]|\\.)*"(?=\s*:)', _keyColor),
    ],
  ),
  // YAML is nearly all keys and values: `true/false/null` are the only words a
  // keyword set can catch, which is why this used to render as a grey slab.
  'yaml': _Lang(
    keywords: const {'true', 'false', 'null', 'yes', 'no', 'on', 'off'},
    lineComments: _hashLike,
    blockComments: false,
    strings: const ['"', "'"],
    rules: [
      // `key:` at the head of a line (indentation and `- ` don't count).
      _Rule(r'''[A-Za-z_][A-Za-z0-9_.\-/]*(?=\s*:(?:\s|$))''', _keyColor,
          leadOnly: true),
      // "quoted key":
      _Rule(r'''"(?:[^"\\]|\\.)*"(?=\s*:(?:\s|$))''', _keyColor, leadOnly: true),
      // Anchors, aliases and merge keys: &base  *base  <<:
      _Rule(r'[&*][A-Za-z0-9_\-]+', _fnColor),
      // Document markers.
      _Rule(r'^(?:---|\.\.\.)$', _comColor, leadOnly: true),
    ],
  ),
  'sql': const _Lang(
    keywords: {
      'select', 'from', 'where', 'insert', 'update', 'delete', 'create',
      'table', 'drop', 'alter', 'join', 'left', 'right', 'inner', 'outer',
      'full', 'on', 'group', 'by', 'order', 'having', 'limit', 'offset', 'as',
      'and', 'or', 'not', 'null', 'into', 'values', 'set', 'distinct', 'count',
      'sum', 'avg', 'min', 'max', 'primary', 'key', 'foreign', 'references',
      'index', 'view', 'union', 'all', 'is', 'in', 'like', 'between', 'asc',
      'desc', 'default',
    },
    lineComments: ['--'],
    caseInsensitive: true,
  ),
  'bash': const _Lang(
    keywords: {
      'if', 'then', 'else', 'elif', 'fi', 'for', 'while', 'do', 'done', 'case',
      'esac', 'function', 'echo', 'export', 'local', 'return', 'in', 'exit',
      'set', 'cd', 'source', 'alias',
    },
    lineComments: _hashLike,
    blockComments: false,
  ),
  // Two things here are not the defaults, and both would be wrong if left:
  //
  //  * NO BACKTICK in `strings`. In PowerShell the backtick is the ESCAPE
  //    character (`` `n ``, `` `t ``), not a string delimiter — leaving the
  //    default would open a string at the first escape and swallow the rest of
  //    the line as one green blob.
  //  * `caseInsensitive`, because PowerShell genuinely is: `ForEach`, `foreach`
  //    and `FOREACH` are the same keyword, and real scripts mix them freely.
  //
  // `blockComments: false` for the same reason as bash: the generic block
  // comment is `/* */`, and PowerShell's is `<# #>`, which the tokenizer has no
  // way to express yet. A `<# #>` block renders plain rather than wrong.
  'powershell': const _Lang(
    keywords: {
      'if', 'elseif', 'else', 'switch', 'foreach', 'for', 'while', 'do',
      'until', 'break', 'continue', 'return', 'function', 'filter', 'param',
      'begin', 'process', 'end', 'try', 'catch', 'finally', 'throw', 'trap',
      'class', 'enum', 'in', 'exit', 'using', 'workflow', 'data', 'dynamicparam',
      'true', 'false', 'null', 'not', 'and', 'or',
    },
    lineComments: _hashLike,
    blockComments: false,
    strings: ['"', "'"],
    caseInsensitive: true,
  ),
  // CSS has no keywords worth the name — it is selectors, properties and
  // values. It scored 1 coloured character out of 36 before these rules.
  'css': _Lang(
    keywords: const {},
    lineComments: const [],
    strings: const ['"', "'"],
    rules: [
      _Rule(r'@[a-zA-Z-]+', _kwColor), // @media, @import, @keyframes
      _Rule(r'[a-zA-Z-]+(?=\s*:)', _keyColor, leadOnly: true), // property:
      _Rule(r'''[.#]?[A-Za-z_][A-Za-z0-9_\-]*(?=[^;{}]*\{)''', _fnColor,
          leadOnly: true), // a selector, i.e. what precedes the next {
      _Rule(r'[.#&:][A-Za-z_][A-Za-z0-9_\-]*', _fnColor), // .cls #id :hover
    ],
  ),
  // HTML/XML: tags and attribute names carry the structure. Its `<!-- -->`
  // comments are neither the `//` nor the `/* */` the tokenizer knows, so they
  // come in as a rule too.
  'html': _Lang(
    keywords: const {},
    lineComments: const [],
    blockComments: false,
    strings: const ['"', "'"],
    rules: [
      _Rule(r'<!--[\s\S]*?(?:-->|$)', _comColor),
      _Rule(r'<[!/?]?[A-Za-z][A-Za-z0-9:.\-]*', _kwColor), // <div  </div  <?xml
      _Rule(r'/?>', _kwColor),
      _Rule(r'[A-Za-z_:][A-Za-z0-9_:.\-]*(?=\s*=)', _keyColor), // attr=
      _Rule(r'&[a-zA-Z]+;|&#\d+;', _numColor), // &nbsp; &#8212;
    ],
  ),
  'plaintext': const _Lang(keywords: {}, lineComments: [], blockComments: false),
  // Mermaid source (shown while the block is focused; unfocused blocks render
  // as a diagram). Diagram-type and structure words; `%%` line comments.
  'mermaid': const _Lang(
    keywords: {
      'graph', 'flowchart', 'sequenceDiagram', 'classDiagram', 'stateDiagram',
      'erDiagram', 'gantt', 'pie', 'journey', 'mindmap', 'timeline',
      'gitGraph', 'quadrantChart', 'subgraph', 'end', 'participant', 'actor',
      'loop', 'alt', 'else', 'opt', 'par', 'note', 'over', 'TD', 'TB', 'LR',
      'RL', 'BT', 'title', 'section', 'class', 'state', 'direction',
    },
    lineComments: ['%%'],
    blockComments: false,
  ),
};

/// Resolve the effective language: explicit choice, or auto-detected when the
/// choice is null/empty/`auto`.
String resolveCodeLanguage(String code, String? selected) {
  final choice = canonicalCodeLanguage(selected ?? '');
  if (choice.isNotEmpty && choice != 'auto') return choice;
  return detectLanguage(code);
}

/// The language whose *structural syntax* the code plainly uses, or null when
/// nothing is conclusive.
///
/// Deliberately much stricter than [detectLanguage]: every signature here is
/// syntax that would be an error in most other languages, not a word that
/// merely leans one way (`echo`, `print(`, `:=` are all far too weak to appear
/// here). Only used to overrule a language a *pasted* block claims about
/// itself — see `retagMislabeledFences` — so a false positive silently
/// mislabels someone's code, and the bar is set accordingly. Returning null is
/// always the safe answer.
String? strongLanguageSignature(String code) {
  bool has(String p) => RegExp(p, multiLine: true).hasMatch(code);

  // A shebang says what the file IS — nothing outranks it.
  if (has(r'^#!.*\b(bash|sh|zsh|ksh)\b')) return 'bash';
  if (has(r'^#!.*\bpython[\d.]*\b')) return 'python';
  if (has(r'^#!.*\bnode\b')) return 'javascript';

  // `def f(...):` / `class C(...):` / `elif`: a colon-terminated block header.
  if (has(r'^\s*(?:async\s+)?def\s+\w+\s*\([^)]*\)\s*(?:->[^:]+)?:') ||
      has(r'^\s*elif\s+.+:\s*$') ||
      has(r'^\s*class\s+\w+\s*(?:\([\w., ]*\))?\s*:\s*$')) {
    return 'python';
  }
  if (has(r'\bfn\s+\w+\s*(?:<[^>]*>)?\s*\([^)]*\)') &&
      (has(r'\blet\s+mut\b') || has(r'\w+!\s*\(') || has(r'->\s*\w'))) {
    return 'rust';
  }
  if (has(r'^\s*package\s+\w+\s*$') && has(r'^\s*func\s+\w+\s*\(')) {
    return 'go';
  }
  // Deliberately absent: `#include` (C and C++ share it, so it cannot tell them
  // apart and would retag cpp→c), and `public class` (C#, Java and Kotlin all
  // have it). A signature that can't name ONE language doesn't belong here.
  return null;
}

/// Heuristic language detection from content signatures.
String detectLanguage(String code) {
  final c = code;
  if (c.isEmpty) return 'plaintext';
  bool has(String pattern) => RegExp(pattern).hasMatch(c);
  bool hasLine(String pattern) => RegExp(pattern, multiLine: true).hasMatch(c);

  // Structural syntax outranks every heuristic below — a shebang, a `def f():`.
  final strong = strongLanguageSignature(c);
  if (strong != null) return strong;

  if (has(r'^\s*<\?xml') || has(r'<\w+[^>]*>.*</\w+>') || has(r'<!DOCTYPE')) {
    return 'html';
  }
  // CSS before YAML: a rule body is full of `prop: value;` lines, which are
  // indistinguishable from YAML keys once you're inside the braces.
  if (has(r'\{[^{}]*[\w-]+\s*:\s*[^;{}]+;') || hasLine(r'^\s*@(?:media|import|keyframes|font-face)\b')) {
    return 'css';
  }
  // YAML — and this was missing entirely, so a pasted config resolved to
  // `plaintext` even on `auto`. It has no keywords to key off: `key:` at the
  // head of a line IS the language. Two of them, or a document marker.
  //
  // Placed above Python's weaker rules on purpose: a compose file's
  // `command: python -c "print(1)"` would otherwise trip Python's `print(`.
  // Python's real signature (`def f():`) already won above, via [strong].
  if (hasLine(r'^---\s*$') ||
      RegExp(r'^[ \t]*-?[ \t]*[\w.-]+:(?:[ \t]|$)', multiLine: true)
              .allMatches(c)
              .length >=
          2) {
    return 'yaml';
  }
  if (has(r'\bfn\s+\w+') && (c.contains('->') || c.contains('let mut') || c.contains('println!'))) {
    return 'rust';
  }
  if (has(r'\bfunc\s+\w+') || has(r'\bpackage\s+main') || c.contains(':=')) {
    return 'go';
  }
  if (has(r'\bdef\s+\w+\s*\(') || has(r'^\s*import\s+\w+\s*$') || c.contains('print(')) {
    return 'python';
  }
  if (c.contains('#include') || has(r'\bstd::')) {
    return c.contains('std::') || c.contains('template') ? 'cpp' : 'c';
  }
  if (has(r'\b(public|private)\s+(static\s+)?(class|void|int)\b') || c.contains('System.out')) {
    return 'java';
  }
  if (has(r'\b(SELECT|INSERT|UPDATE|DELETE|CREATE)\b', )) return 'sql';
  if (has(r'\b(const|let|var|function)\b') && (c.contains('=>') || c.contains('){'))) {
    return c.contains(': ') && (c.contains('interface ') || c.contains(': string') || c.contains(': number'))
        ? 'typescript'
        : 'javascript';
  }
  if (has(r'\bvoid\s+main\b') || c.contains('Widget build(')) return 'dart';
  if (c.trimLeft().startsWith('{') || c.trimLeft().startsWith('[')) return 'json';
  if (has(r'^\s*#!/.*sh\b') || has(r'\becho\s+')) return 'bash';
  return 'plaintext';
}

/// Build a colored [TextSpan] for [code] under [language], over [base] style.
TextSpan buildCodeSpan(String code, String language, TextStyle base) {
  final name = canonicalCodeLanguage(language);
  final lang = _langs[name];
  if (lang == null || name == 'plaintext') {
    return TextSpan(text: code, style: base);
  }

  final spans = <TextSpan>[];
  final n = code.length;
  var i = 0;
  final buffer = StringBuffer();

  void flushPlain() {
    if (buffer.isNotEmpty) {
      spans.add(TextSpan(text: buffer.toString(), style: base));
      buffer.clear();
    }
  }

  void emit(String text, Color color) {
    flushPlain();
    spans.add(TextSpan(text: text, style: base.copyWith(color: color)));
  }

  bool isIdentStart(String ch) => RegExp(r'[A-Za-z_]').hasMatch(ch);
  bool isIdent(String ch) => RegExp(r'[A-Za-z0-9_]').hasMatch(ch);
  bool isDigit(String ch) => RegExp(r'[0-9]').hasMatch(ch);

  // Whether only indentation (and YAML's `- ` marker) has been seen on this
  // line so far — what makes a `key:` a key. See [_Rule.leadOnly].
  var lineLead = true;

  while (i < n) {
    final ch = code[i];
    final rest = code.substring(i);
    final atLead = lineLead;
    if (ch == '\n') {
      lineLead = true;
    } else if (ch != ' ' && ch != '\t' && ch != '-') {
      lineLead = false;
    }

    // Structural rules run first: an HTML `<!-- -->` must beat the string
    // scanner on the quotes inside it, and a JSON key must be claimed before
    // it is tokenized as just another string.
    var matchedRule = false;
    for (final rule in lang.rules) {
      if (rule.leadOnly && !atLead) continue;
      final m = rule.pattern.matchAsPrefix(code, i);
      if (m == null) continue;
      final text = (m.groupCount >= 1 ? m.group(1) : m.group(0)) ?? '';
      if (text.isEmpty) continue;
      emit(text, rule.color);
      i += text.length;
      lineLead = false;
      matchedRule = true;
      break;
    }
    if (matchedRule) continue;

    // Block comments
    if (lang.blockComments && rest.startsWith('/*')) {
      final end = code.indexOf('*/', i + 2);
      final stop = end < 0 ? n : end + 2;
      emit(code.substring(i, stop), _comColor);
      i = stop;
      continue;
    }

    // Line comments
    var matchedComment = false;
    for (final prefix in lang.lineComments) {
      if (rest.startsWith(prefix)) {
        final nl = code.indexOf('\n', i);
        final stop = nl < 0 ? n : nl;
        emit(code.substring(i, stop), _comColor);
        i = stop;
        matchedComment = true;
        break;
      }
    }
    if (matchedComment) continue;

    // Strings
    if (lang.strings.contains(ch)) {
      var j = i + 1;
      while (j < n) {
        if (code[j] == '\\') {
          j += 2;
          continue;
        }
        if (code[j] == ch) {
          j += 1;
          break;
        }
        if (code[j] == '\n') break; // unterminated; stop at line end
        j += 1;
      }
      emit(code.substring(i, j.clamp(0, n)), _strColor);
      i = j.clamp(0, n);
      continue;
    }

    // Numbers
    if (isDigit(ch)) {
      var j = i + 1;
      while (j < n && RegExp(r'[0-9a-fA-FxX._]').hasMatch(code[j])) {
        j += 1;
      }
      emit(code.substring(i, j), _numColor);
      i = j;
      continue;
    }

    // Identifiers / keywords
    if (isIdentStart(ch)) {
      var j = i + 1;
      while (j < n && isIdent(code[j])) {
        j += 1;
      }
      final word = code.substring(i, j);
      final probe = lang.caseInsensitive ? word.toLowerCase() : word;
      final isKeyword = lang.caseInsensitive
          ? lang.keywords.contains(probe)
          : lang.keywords.contains(word);
      if (isKeyword) {
        emit(word, _kwColor);
      } else if (j < n && code[j] == '(') {
        emit(word, _fnColor);
      } else {
        buffer.write(word);
      }
      i = j;
      continue;
    }

    buffer.write(ch);
    i += 1;
  }

  flushPlain();
  return TextSpan(style: base, children: spans);
}
