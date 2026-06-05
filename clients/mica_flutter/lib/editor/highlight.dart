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

class _Lang {
  const _Lang({
    this.keywords = const {},
    this.lineComments = const ['//'],
    this.blockComments = true,
    this.strings = const ['"', "'", '`'],
    this.caseInsensitive = false,
  });

  final Set<String> keywords;
  final List<String> lineComments;
  final bool blockComments;
  final List<String> strings;
  final bool caseInsensitive;
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
  'json': const _Lang(
    keywords: {'true', 'false', 'null'},
    lineComments: [],
    blockComments: false,
    strings: ['"'],
  ),
  'yaml': const _Lang(
    keywords: {'true', 'false', 'null', 'yes', 'no'},
    lineComments: _hashLike,
    blockComments: false,
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
  'css': const _Lang(
    keywords: {},
    lineComments: [],
    strings: ['"', "'"],
  ),
  'html': const _Lang(keywords: {}, lineComments: [], blockComments: false),
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
  final choice = selected?.trim().toLowerCase() ?? '';
  if (choice.isNotEmpty && choice != 'auto') return choice;
  return detectLanguage(code);
}

/// Heuristic language detection from content signatures.
String detectLanguage(String code) {
  final c = code;
  if (c.isEmpty) return 'plaintext';
  bool has(String pattern) => RegExp(pattern).hasMatch(c);

  if (has(r'^\s*<\?xml') || has(r'<\w+[^>]*>.*</\w+>') || has(r'<!DOCTYPE')) {
    return 'html';
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
  final lang = _langs[language];
  if (lang == null || language == 'plaintext') {
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

  while (i < n) {
    final ch = code[i];
    final rest = code.substring(i);

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
