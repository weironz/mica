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
  // Application languages people paste next to the ones above.
  'kotlin',
  'swift',
  'csharp',
  'php',
  'ruby',
  'objective-c',
  'lua',
  'perl',
  'r',
  'scala',
  'groovy',
  'elixir',
  'haskell',
  'zig',
  // Config / markup / ops formats — the bulk of what actually lands in notes.
  'dockerfile',
  'xml',
  'toml',
  'ini',
  'diff',
  'markdown',
  'graphql',
  'protobuf',
  'nginx',
  'makefile',
  'latex',
  'nix',
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
  'xhtml': 'html',
  // `xml` used to alias to `html`. It now has its own entry: the tag rules are
  // the same, but XML also has `<![CDATA[ ]]>` and `<?pi ?>`, and an XML block
  // labelled "html" in the picker reads as a mistake.
  'xsd': 'xml',
  'xsl': 'xml',
  'xslt': 'xml',
  'svg': 'xml',
  'plist': 'xml',
  'rss': 'xml',
  'pom': 'xml',
  'jsonc': 'json',
  'json5': 'json',
  'sqlite': 'sql',
  'psql': 'sql',
  'postgres': 'sql',
  'postgresql': 'sql',
  'mysql': 'sql',
  'kt': 'kotlin',
  'kts': 'kotlin',
  'cs': 'csharp',
  'c#': 'csharp',
  'dotnet': 'csharp',
  'php3': 'php',
  'php8': 'php',
  'rb': 'ruby',
  'ruby-script': 'ruby',
  'gemfile': 'ruby',
  'objc': 'objective-c',
  'objectivec': 'objective-c',
  'obj-c': 'objective-c',
  'mm': 'objective-c',
  'pl': 'perl',
  'pm': 'perl',
  'rlang': 'r',
  'rscript': 'r',
  'sc': 'scala',
  'gradle': 'groovy',
  'ex': 'elixir',
  'exs': 'elixir',
  'hs': 'haskell',
  'lhs': 'haskell',
  'docker': 'dockerfile',
  'containerfile': 'dockerfile',
  'cfg': 'ini',
  'editorconfig': 'ini',
  'patch': 'diff',
  'udiff': 'diff',
  'md': 'markdown',
  'mkd': 'markdown',
  'mdown': 'markdown',
  'gql': 'graphql',
  'proto': 'protobuf',
  'proto3': 'protobuf',
  // `conf` is ambiguous in the abstract, but in practice a fence labelled
  // `conf` is nginx far more often than anything else.
  'conf': 'nginx',
  'nginxconf': 'nginx',
  'make': 'makefile',
  'mk': 'makefile',
  'gnumakefile': 'makefile',
  'tex': 'latex',
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

  // ---------------------------------------------------------------------
  // Application languages.
  //
  // The recurring trap in this block is `strings`, whose default is
  // `['"', "'", '`']`. A backtick is a string delimiter in almost nothing
  // (JS/Go/Dart aside): in Kotlin it quotes an identifier, in Ruby/Perl it
  // runs a subprocess, in Markdown it opens inline code. Leaving the default
  // opens a "string" at the first one and paints the rest of the line green.
  // Every entry below therefore states `strings` explicitly.
  // ---------------------------------------------------------------------

  // Backtick excluded: in Kotlin it quotes an identifier (`` `is` ``, and JUnit
  // test names are written that way constantly), never a string.
  'kotlin': const _Lang(
    keywords: {
      'fun', 'val', 'var', 'class', 'object', 'interface', 'data', 'sealed',
      'enum', 'companion', 'init', 'constructor', 'override', 'open',
      'abstract', 'private', 'protected', 'public', 'internal', 'suspend',
      'inline', 'reified', 'return', 'if', 'else', 'when', 'for', 'while',
      'do', 'break', 'continue', 'try', 'catch', 'finally', 'throw', 'import',
      'package', 'in', 'is', 'as', 'by', 'out', 'null', 'true', 'false',
      'this', 'super', 'lateinit', 'typealias', 'vararg', 'operator', 'infix',
      'const', 'annotation', 'crossinline', 'noinline', 'expect', 'actual',
    },
    strings: ['"', "'"],
  ),
  // Only `"` — Swift has no single-quoted literal at all (a character is
  // `"a"` typed as Character), so `'` is never a delimiter.
  'swift': const _Lang(
    keywords: {
      'func', 'let', 'var', 'class', 'struct', 'enum', 'protocol', 'extension',
      'init', 'deinit', 'guard', 'if', 'else', 'switch', 'case', 'default',
      'for', 'while', 'repeat', 'return', 'break', 'continue', 'in', 'is',
      'as', 'throw', 'throws', 'rethrows', 'try', 'catch', 'defer', 'import',
      'public', 'private', 'internal', 'fileprivate', 'open', 'static',
      'final', 'lazy', 'weak', 'unowned', 'mutating', 'override', 'where',
      'associatedtype', 'typealias', 'some', 'any', 'async', 'await', 'actor',
      'inout', 'subscript', 'willSet', 'didSet', 'nil', 'true', 'false',
      'self', 'Self', 'convenience', 'required', 'indirect',
    },
    strings: ['"'],
  ),
  'csharp': const _Lang(
    keywords: {
      'using', 'namespace', 'class', 'struct', 'interface', 'enum', 'record',
      'public', 'private', 'protected', 'internal', 'static', 'readonly',
      'const', 'void', 'int', 'long', 'short', 'byte', 'char', 'bool',
      'float', 'double', 'decimal', 'string', 'object', 'var', 'dynamic',
      'new', 'return', 'if', 'else', 'switch', 'case', 'default', 'for',
      'foreach', 'while', 'do', 'break', 'continue', 'try', 'catch',
      'finally', 'throw', 'async', 'await', 'get', 'set', 'this', 'base',
      'null', 'true', 'false', 'override', 'virtual', 'abstract', 'sealed',
      'partial', 'in', 'out', 'ref', 'is', 'as', 'typeof', 'nameof', 'yield',
      'lock', 'params', 'delegate', 'event', 'operator', 'when', 'where',
    },
    strings: ['"', "'"],
  ),
  // `#` is a second line comment in PHP alongside `//`.
  'php': _Lang(
    keywords: const {
      'function', 'class', 'interface', 'trait', 'extends', 'implements',
      'public', 'private', 'protected', 'static', 'const', 'return', 'if',
      'else', 'elseif', 'endif', 'foreach', 'endforeach', 'as', 'for',
      'while', 'do', 'switch', 'case', 'default', 'break', 'continue', 'try',
      'catch', 'finally', 'throw', 'new', 'echo', 'print', 'require',
      'require_once', 'include', 'include_once', 'namespace', 'use', 'array',
      'null', 'true', 'false', 'this', 'abstract', 'final', 'global', 'isset',
      'unset', 'instanceof', 'fn', 'match', 'enum', 'readonly', 'yield',
    },
    lineComments: const ['//', '#'],
    strings: const ['"', "'"],
    rules: [
      _Rule(r'<\?(?:php|=)?|\?>', _kwColor),
      _Rule(r'\$[A-Za-z_][A-Za-z0-9_]*', _keyColor),
    ],
  ),
  // Backtick excluded: `` `cmd` `` runs a subprocess in Ruby. `=begin/=end`
  // block comments are neither `//` nor `/* */`, so they render plain rather
  // than wrong.
  'ruby': _Lang(
    keywords: const {
      'def', 'end', 'class', 'module', 'if', 'elsif', 'else', 'unless',
      'case', 'when', 'while', 'until', 'for', 'in', 'do', 'begin', 'rescue',
      'ensure', 'raise', 'return', 'yield', 'next', 'break', 'redo', 'retry',
      'then', 'self', 'nil', 'true', 'false', 'and', 'or', 'not', 'require',
      'require_relative', 'include', 'extend', 'attr_accessor', 'attr_reader',
      'attr_writer', 'lambda', 'proc', 'puts', 'new', 'super', 'alias',
      'undef', 'private', 'public', 'protected', 'defined',
    },
    lineComments: _hashLike,
    blockComments: false,
    strings: const ['"', "'"],
    rules: [
      _Rule(r'@@?[A-Za-z_][A-Za-z0-9_]*', _keyColor), // @ivar / @@cvar
      _Rule(r':[A-Za-z_][A-Za-z0-9_]*[?!]?', _fnColor), // :symbol
    ],
  ),
  // `@interface` / `#import` are the shape of the language; the plain C
  // keyword set alone leaves an Objective-C header nearly grey.
  'objective-c': _Lang(
    keywords: const {
      'id', 'self', 'super', 'nil', 'YES', 'NO', 'BOOL', 'instancetype',
      'void', 'int', 'char', 'float', 'double', 'long', 'short', 'unsigned',
      'signed', 'const', 'static', 'extern', 'struct', 'union', 'enum',
      'typedef', 'return', 'if', 'else', 'for', 'while', 'do', 'switch',
      'case', 'break', 'continue', 'sizeof', 'inline', 'NULL',
    },
    strings: const ['"'],
    rules: [
      _Rule(r'@[A-Za-z_][A-Za-z0-9_]*', _kwColor), // @interface @property @end
      _Rule(r'#[a-z]+', _kwColor), // #import #define
    ],
  ),
  // `--` line comments; `--[[ ]]` blocks are not `/* */`, hence false.
  'lua': const _Lang(
    keywords: {
      'and', 'break', 'do', 'else', 'elseif', 'end', 'false', 'for',
      'function', 'goto', 'if', 'in', 'local', 'nil', 'not', 'or', 'repeat',
      'return', 'then', 'true', 'until', 'while', 'self', 'require', 'pairs',
      'ipairs', 'print', 'pcall', 'setmetatable',
    },
    lineComments: ['--'],
    blockComments: false,
    strings: ['"', "'"],
  ),
  // Backtick excluded (subprocess, as in Ruby). Sigils carry the structure.
  'perl': _Lang(
    keywords: const {
      'my', 'our', 'local', 'sub', 'package', 'use', 'no', 'require', 'if',
      'elsif', 'else', 'unless', 'while', 'until', 'for', 'foreach', 'do',
      'last', 'next', 'redo', 'return', 'die', 'warn', 'print', 'printf',
      'say', 'chomp', 'chop', 'defined', 'undef', 'ref', 'bless', 'wantarray',
      'eval', 'qw', 'and', 'or', 'not', 'eq', 'ne', 'lt', 'gt', 'le', 'ge',
      'cmp', 'keys', 'values', 'push', 'pop', 'shift', 'unshift', 'split',
      'join', 'map', 'grep', 'sort', 'scalar', 'exists', 'delete',
    },
    lineComments: _hashLike,
    blockComments: false,
    strings: const ['"', "'"],
    rules: [
      _Rule(r'[$@%][$#]?[A-Za-z_][A-Za-z0-9_:]*', _keyColor), // $x @a %h
    ],
  ),
  // Backtick excluded: in R it quotes a non-syntactic name (`` `my col` ``).
  'r': const _Lang(
    keywords: {
      'function', 'if', 'else', 'for', 'while', 'repeat', 'break', 'next',
      'return', 'TRUE', 'FALSE', 'NULL', 'NA', 'Inf', 'NaN', 'library',
      'require', 'in', 'invisible', 'switch', 'stop', 'warning', 'print',
    },
    lineComments: _hashLike,
    blockComments: false,
    strings: ['"', "'"],
  ),
  // Only `"` — `'foo` is a Symbol literal in Scala and would otherwise open an
  // unterminated string and eat the line.
  'scala': const _Lang(
    keywords: {
      'def', 'val', 'var', 'class', 'object', 'trait', 'case', 'match',
      'extends', 'with', 'override', 'implicit', 'import', 'package', 'new',
      'if', 'else', 'for', 'while', 'do', 'yield', 'try', 'catch', 'finally',
      'throw', 'return', 'sealed', 'abstract', 'final', 'private',
      'protected', 'lazy', 'type', 'this', 'super', 'null', 'true', 'false',
      'given', 'using', 'enum', 'then', 'forSome',
    },
    strings: ['"'],
  ),
  'groovy': const _Lang(
    keywords: {
      'def', 'class', 'interface', 'trait', 'enum', 'extends', 'implements',
      'import', 'package', 'new', 'return', 'if', 'else', 'for', 'while',
      'do', 'switch', 'case', 'default', 'break', 'continue', 'try', 'catch',
      'finally', 'throw', 'throws', 'public', 'private', 'protected',
      'static', 'final', 'void', 'int', 'long', 'boolean', 'String', 'true',
      'false', 'null', 'this', 'super', 'as', 'in', 'assert', 'it',
    },
    strings: ['"', "'"],
  ),
  'elixir': _Lang(
    keywords: const {
      'def', 'defp', 'defmodule', 'defstruct', 'defprotocol', 'defimpl',
      'defmacro', 'defdelegate', 'defexception', 'do', 'end', 'if', 'else',
      'unless', 'cond', 'case', 'when', 'fn', 'receive', 'after', 'rescue',
      'catch', 'try', 'raise', 'throw', 'import', 'alias', 'require', 'use',
      'with', 'for', 'and', 'or', 'not', 'in', 'nil', 'true', 'false',
    },
    lineComments: _hashLike,
    blockComments: false,
    // `'…'` is a charlist — a real literal, so both quotes stay.
    strings: const ['"', "'"],
    rules: [
      _Rule(r':[A-Za-z_][A-Za-z0-9_]*[?!]?', _fnColor), // :atom
      _Rule(r'@[a-z_][A-Za-z0-9_]*', _keyColor), // @moduledoc @spec
    ],
  ),
  // `--` comments; `{- -}` blocks are not `/* */`. Only `"` for strings: an
  // apostrophe is a legal identifier character in Haskell (`x'`, `foldl'`),
  // so treating it as a quote would swallow the rest of the line.
  'haskell': const _Lang(
    keywords: {
      'module', 'where', 'import', 'qualified', 'as', 'hiding', 'let', 'in',
      'do', 'case', 'of', 'if', 'then', 'else', 'data', 'newtype', 'type',
      'class', 'instance', 'deriving', 'default', 'infix', 'infixl', 'infixr',
      'foreign', 'forall', 'True', 'False', 'Nothing', 'Just', 'IO', 'Maybe',
      'Either', 'Left', 'Right',
    },
    lineComments: ['--'],
    blockComments: false,
    strings: ['"'],
  ),
  // Zig has NO block comments at all (`//` only, by design), so leaving the
  // generic `/* */` on would be inventing syntax.
  'zig': _Lang(
    keywords: const {
      'const', 'var', 'fn', 'pub', 'return', 'if', 'else', 'switch', 'while',
      'for', 'break', 'continue', 'defer', 'errdefer', 'try', 'catch',
      'orelse', 'unreachable', 'struct', 'enum', 'union', 'error',
      'comptime', 'inline', 'export', 'extern', 'test', 'and', 'or', 'null',
      'undefined', 'true', 'false', 'usingnamespace', 'async', 'await',
      'suspend', 'resume', 'anytype', 'noreturn', 'void', 'bool', 'u8',
      'u16', 'u32', 'u64', 'i8', 'i16', 'i32', 'i64', 'usize', 'isize',
      'f32', 'f64', 'align', 'packed', 'opaque', 'threadlocal', 'volatile',
      'callconv', 'anyerror', 'linksection', 'noalias',
    },
    blockComments: false,
    strings: const ['"', "'"],
    rules: [
      _Rule(r'@[A-Za-z_][A-Za-z0-9_]*', _fnColor), // @import @intCast
    ],
  ),

  // ---------------------------------------------------------------------
  // Config / markup / ops formats.
  // ---------------------------------------------------------------------

  // Instructions are conventionally shouted but the parser accepts any case
  // (`from alpine` is valid), and real Dockerfiles mix them — hence
  // caseInsensitive, as for SQL and PowerShell.
  'dockerfile': const _Lang(
    keywords: {
      'from', 'run', 'cmd', 'label', 'maintainer', 'expose', 'env', 'add',
      'copy', 'entrypoint', 'volume', 'user', 'workdir', 'arg', 'onbuild',
      'stopsignal', 'healthcheck', 'shell', 'as',
    },
    lineComments: _hashLike,
    blockComments: false,
    strings: ['"', "'"],
    caseInsensitive: true,
  ),
  // Same shape as `html` — tags and attributes are the structure — plus the
  // two things XML has and HTML does not: `<![CDATA[ ]]>` and `<?pi ?>`.
  'xml': _Lang(
    keywords: const {},
    lineComments: const [],
    blockComments: false,
    strings: const ['"', "'"],
    rules: [
      _Rule(r'<!--[\s\S]*?(?:-->|$)', _comColor),
      _Rule(r'<!\[CDATA\[[\s\S]*?(?:\]\]>|$)', _strColor),
      _Rule(r'<[!/?]?[A-Za-z_][A-Za-z0-9:._\-]*', _kwColor),
      _Rule(r'\??/?>', _kwColor),
      _Rule(r'[A-Za-z_:][A-Za-z0-9_:.\-]*(?=\s*=)', _keyColor),
      _Rule(r'&[a-zA-Z]+;|&#\d+;', _numColor),
    ],
  ),
  // Tables and keys, like YAML — a keyword set can only reach `true`/`false`.
  'toml': _Lang(
    keywords: const {'true', 'false'},
    lineComments: _hashLike,
    blockComments: false,
    // `'…'` is TOML's literal string. No backtick.
    strings: const ['"', "'"],
    rules: [
      _Rule(r'\[\[?[^\]\n]*\]\]?', _fnColor, leadOnly: true), // [tbl] [[arr]]
      _Rule(r'"(?:[^"\\]|\\.)*"(?=\s*=)', _keyColor, leadOnly: true),
      _Rule(r'[A-Za-z_][A-Za-z0-9_.\-]*(?=\s*=)', _keyColor, leadOnly: true),
    ],
  ),
  // INI takes BOTH `;` and `#` as line comments — `;` is the older, and still
  // the more common one in .gitconfig / php.ini / systemd-adjacent files.
  'ini': _Lang(
    keywords: const {'true', 'false', 'yes', 'no', 'on', 'off'},
    lineComments: const [';', '#'],
    blockComments: false,
    strings: const ['"', "'"],
    caseInsensitive: true,
    rules: [
      _Rule(r'\[[^\]\n]*\]', _fnColor, leadOnly: true), // [section]
      _Rule(r'[A-Za-z_][A-Za-z0-9_.\-]*(?=\s*=)', _keyColor, leadOnly: true),
    ],
  ),
  // A diff has no lexical structure whatsoever — it is line-oriented, and the
  // ONLY thing that matters is the first column. Every generic mechanism is
  // therefore off, including `strings`: quotes in a diff are just whatever the
  // patched file happened to contain, and pairing them across `+`/`-` lines
  // would paint arbitrary regions green.
  'diff': _Lang(
    keywords: const {},
    lineComments: const [],
    blockComments: false,
    strings: const [],
    rules: [
      // `---`/`+++` first: they are also `-`/`+` lines.
      _Rule(r'(?:\+\+\+|---)[^\n]*', _keyColor, leadOnly: true),
      _Rule(r'@@[^\n]*', _fnColor, leadOnly: true),
      _Rule(r'(?:diff|index|similarity|rename|new file|deleted file)[^\n]*',
          _comColor, leadOnly: true),
      _Rule(r'\+[^\n]*', _strColor, leadOnly: true), // added
      _Rule(r'-[^\n]*', _numColor, leadOnly: true), // removed
    ],
  ),
  // Markdown is punctuation, not words. Note `strings: []`: a backtick opens
  // inline code (handled by a rule, coloured as a string), and an apostrophe
  // in ordinary prose ("don't") would otherwise open a string and swallow the
  // rest of the paragraph.
  'markdown': _Lang(
    keywords: const {},
    lineComments: const [],
    blockComments: false,
    strings: const [],
    rules: [
      _Rule(r'#{1,6}[^\n]*', _kwColor, leadOnly: true), // # Heading
      _Rule(r'>[^\n]*', _comColor, leadOnly: true), // > quote
      _Rule(r'(?:```|~~~)[^\n]*', _keyColor), // fence
      _Rule(r'`[^`\n]+`', _strColor), // `inline code`
      _Rule(r'\*\*[^\n]*?\*\*|__[^\n]*?__', _kwColor), // **bold**
      _Rule(r'\*[^*\n]+\*|_[^_\n]+_', _kwColor), // *italic*
      _Rule(r'!?\[[^\]\n]*\]\([^)\n]*\)', _fnColor), // [text](url)
      _Rule(r'[-*+](?=\s)', _numColor, leadOnly: true), // list bullet
    ],
  ),
  'graphql': _Lang(
    keywords: const {
      'query', 'mutation', 'subscription', 'fragment', 'on', 'type', 'input',
      'interface', 'union', 'enum', 'scalar', 'schema', 'extend',
      'implements', 'directive', 'repeatable', 'true', 'false', 'null',
    },
    lineComments: _hashLike,
    blockComments: false,
    strings: const ['"'],
    rules: [
      _Rule(r'[A-Za-z_][A-Za-z0-9_]*(?=\s*:)', _keyColor), // field:
      _Rule(r'[$@][A-Za-z_][A-Za-z0-9_]*', _fnColor), // $var / @directive
    ],
  ),
  'protobuf': const _Lang(
    keywords: {
      'syntax', 'package', 'import', 'message', 'enum', 'service', 'rpc',
      'returns', 'repeated', 'optional', 'required', 'reserved', 'oneof',
      'map', 'extend', 'extensions', 'option', 'stream', 'public', 'bool',
      'string', 'bytes', 'int32', 'int64', 'uint32', 'uint64', 'sint32',
      'sint64', 'fixed32', 'fixed64', 'float', 'double', 'true', 'false',
    },
    strings: ['"', "'"],
  ),
  // nginx.conf is directives and blocks: the first word of a line IS the
  // language, exactly like a YAML key.
  'nginx': _Lang(
    keywords: const {'on', 'off', 'true', 'false'},
    lineComments: _hashLike,
    blockComments: false,
    strings: const ['"', "'"],
    rules: [
      _Rule(r'[a-z_][a-z0-9_]*(?=[\s;{])', _keyColor, leadOnly: true),
      _Rule(r'\$[A-Za-z_][A-Za-z0-9_]*', _fnColor), // $host $remote_addr
    ],
  ),
  // Targets and variable expansions. `$(…)` is claimed before anything else,
  // otherwise the `$` and the name split into unrelated tokens.
  'makefile': _Lang(
    keywords: const {
      'ifeq', 'ifneq', 'ifdef', 'ifndef', 'else', 'endif', 'include',
      'define', 'endef', 'export', 'unexport', 'override', 'vpath',
    },
    lineComments: _hashLike,
    blockComments: false,
    strings: const ['"', "'"],
    rules: [
      _Rule(r'\$[({][^)}\n]*[)}]|\$\$?[A-Za-z@<^?*]', _kwColor),
      // `target:` — but not `VAR :=`, hence the (?!=).
      _Rule(r'\.?[A-Za-z0-9_%./\-]+(?=\s*:(?!=))', _fnColor, leadOnly: true),
      _Rule(r'[A-Za-z_][A-Za-z0-9_]*(?=\s*[:?+!]?=)', _keyColor,
          leadOnly: true),
    ],
  ),
  // `%` line comments and NO block comment form at all. `strings: []` because
  // LaTeX has no string literal — `"` is a plain character and the idiomatic
  // quotes are `` ` `` and `'`, which as delimiters would eat whole paragraphs.
  'latex': _Lang(
    keywords: const {},
    lineComments: const ['%'],
    blockComments: false,
    strings: const [],
    rules: [
      _Rule(r'\\(?:begin|end)\{[^}\n]*\}', _fnColor),
      _Rule(r'\\[A-Za-z@]+\*?', _kwColor), // \section \textbf
      _Rule(r'\\[^A-Za-z\s]', _kwColor), // \% \& \\
      _Rule(r'[\$&]', _keyColor), // math toggle, alignment tab
    ],
  ),
  // Nix genuinely uses `/* */` (unlike every other `#`-comment language here),
  // so blockComments stays on. Only `"`: the multi-line form is `'' … ''`, and
  // treating a lone `'` as a delimiter would break it in half.
  'nix': const _Lang(
    keywords: {
      'let', 'in', 'rec', 'with', 'inherit', 'if', 'then', 'else', 'assert',
      'or', 'import', 'builtins', 'true', 'false', 'null', 'derivation',
      'mkDerivation', 'callPackage', 'fetchurl', 'fetchFromGitHub',
    },
    lineComments: _hashLike,
    strings: ['"'],
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
  // Shell: real pasted sessions carry strong signals even without a shebang —
  // a prompt line (`user@host:…$`/`#`), a pipe into a text tool, a redirect, or
  // a common admin/CLI command at line start. The old check only knew `echo` and
  // shebangs, so `nvidia-smi | grep …`, `systemctl is-active …`, or a
  // `root@host:~# …` session fell through to plaintext (no highlighting).
  if (hasLine(r'^\s*#!.*\bsh\b') ||
      hasLine(r'[\w.-]+@[\w.-]+:.*[$#]') ||
      has(r'\becho\s+') ||
      has(r'\|\s*(?:grep|awk|sed|head|tail|xargs|sort|uniq|wc|cut|tr|less)\b') ||
      has(r'2>&1') ||
      has(r'>\s*/dev/null') ||
      hasLine(
        r'^\s*(?:sudo|apt|apt-get|yum|dnf|systemctl|service|journalctl|docker|'
        r'kubectl|helm|git|curl|wget|ssh|scp|rsync|tar|chmod|chown|mkdir|export|'
        r'source|pip3?|python3?|nvidia-smi|dcgmi)\b',
      )) {
    return 'bash';
  }
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
