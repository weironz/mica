import 'package:flutter/widgets.dart';

/// Inline rich-text marks stored additively over a block's plain text (in
/// `data.marks`). The text stays clean; a mark is a `[start, end)` range with a
/// type (and an href for links). See docs/editor-engine.md.
class Mark {
  Mark(this.start, this.end, this.type, {this.href, this.title});

  final int start;
  final int end;
  final String type; // bold | italic | code | strike | link
  final String? href;

  /// Optional link title (`[t](url "title")`) — carried for GFM fidelity.
  final String? title;

  Mark shifted(int delta) =>
      Mark(start + delta, end + delta, type, href: href, title: title);

  Map<String, dynamic> toJson() => {
    'start': start,
    'end': end,
    'type': type,
    if (href != null) 'href': href,
    if (title != null) 'title': title,
  };

  static const types = {'bold', 'italic', 'code', 'strike', 'link'};
}

List<Mark> marksFromData(Map<String, dynamic> data) {
  final raw = data['marks'];
  if (raw is! List) return [];
  final marks = <Mark>[];
  for (final m in raw) {
    if (m is Map) {
      final start = (m['start'] as num?)?.toInt();
      final end = (m['end'] as num?)?.toInt();
      final type = m['type'] as String?;
      if (start != null && end != null && type != null && end > start) {
        marks.add(Mark(start, end, type,
            href: m['href'] as String?, title: m['title'] as String?));
      }
    }
  }
  return marks;
}

List<Map<String, dynamic>> marksToJson(List<Mark> marks) =>
    [for (final m in marks) m.toJson()];

const Color _codeColor = Color(0xFFB91C1C);
const Color _codeBg = Color(0x14B91C1C);
const Color _linkColor = Color(0xFF2563EB);

/// Build a styled [TextSpan] for [text] with [marks] applied over [base].
TextSpan buildMarkedSpan(String text, List<Mark> marks, TextStyle base) {
  if (text.isEmpty) return TextSpan(text: '​', style: base);
  if (marks.isEmpty) return TextSpan(text: text, style: base);

  final len = text.length;
  final points = <int>{0, len};
  for (final m in marks) {
    points.add(m.start.clamp(0, len));
    points.add(m.end.clamp(0, len));
  }
  final sorted = points.toList()..sort();

  final children = <TextSpan>[];
  for (var i = 0; i < sorted.length - 1; i++) {
    final a = sorted[i];
    final b = sorted[i + 1];
    if (a >= b) continue;
    final active = marks.where((m) => m.start <= a && m.end >= b);
    var style = base;
    final decorations = <TextDecoration>[];
    for (final m in active) {
      switch (m.type) {
        case 'bold':
          style = style.copyWith(fontWeight: FontWeight.w700);
        case 'italic':
          style = style.copyWith(fontStyle: FontStyle.italic);
        case 'code':
          style = style.copyWith(
            fontFamily: 'monospace',
            color: _codeColor,
            backgroundColor: _codeBg,
          );
        case 'strike':
          decorations.add(TextDecoration.lineThrough);
        case 'link':
          style = style.copyWith(color: _linkColor);
          decorations.add(TextDecoration.underline);
      }
    }
    if (decorations.isNotEmpty) {
      style = style.copyWith(decoration: TextDecoration.combine(decorations));
    }
    children.add(TextSpan(text: text.substring(a, b), style: style));
  }
  return TextSpan(style: base, children: children);
}

/// Whether `[from, to)` is fully covered by a mark of [type].
bool rangeHasMark(List<Mark> marks, int from, int to, String type) {
  return marks.any((m) => m.type == type && m.start <= from && m.end >= to);
}

/// Add or remove [type] over `[from, to)` (toggle decided by the caller).
List<Mark> applyMark(
  List<Mark> marks,
  int from,
  int to,
  String type, {
  String? href,
  required bool add,
}) {
  if (to <= from) return marks;
  final result = <Mark>[];
  for (final m in marks) {
    if (m.type != type || m.end <= from || m.start >= to) {
      result.add(m);
      continue;
    }
    // Trim the overlapping part of the same-type mark out of [from, to).
    if (m.start < from) result.add(Mark(m.start, from, type, href: m.href));
    if (m.end > to) result.add(Mark(to, m.end, type, href: m.href));
  }
  if (add) result.add(Mark(from, to, type, href: href));
  return _normalize(result);
}

/// Shift mark offsets for a text edit that replaced `[editStart, editOldEnd)`
/// with `delta = newLen - oldLen` net characters.
List<Mark> shiftMarks(
  List<Mark> marks,
  int editStart,
  int editOldEnd,
  int delta,
  int newLen,
) {
  int move(int x) {
    if (x <= editStart) return x;
    if (x >= editOldEnd) return x + delta;
    return editStart; // endpoint inside the replaced region collapses
  }

  final result = <Mark>[];
  for (final m in marks) {
    final s = move(m.start).clamp(0, newLen);
    final e = move(m.end).clamp(0, newLen);
    if (e > s) result.add(Mark(s, e, m.type, href: m.href));
  }
  return _normalize(result);
}

/// Marks for the two halves of a text split at [at]: first list keeps
/// `[0, at)` ranges, second gets `[at, …)` ranges rebased to 0. A mark
/// spanning the split is divided.
(List<Mark>, List<Mark>) splitMarks(List<Mark> marks, int at) {
  final before = <Mark>[];
  final after = <Mark>[];
  for (final m in marks) {
    if (m.start < at) {
      final end = m.end < at ? m.end : at;
      if (end > m.start) before.add(Mark(m.start, end, m.type, href: m.href));
    }
    if (m.end > at) {
      final start = m.start > at ? m.start : at;
      if (m.end > start) {
        after.add(Mark(start - at, m.end - at, m.type, href: m.href));
      }
    }
  }
  return (before, after);
}

/// Marks of two concatenated texts: [a]'s marks plus [b]'s shifted past the
/// [junction] (the first text's length).
List<Mark> concatMarks(List<Mark> a, List<Mark> b, int junction) =>
    _normalize([...a, for (final m in b) m.shifted(junction)]);

List<Mark> _normalize(List<Mark> marks) {
  if (marks.isEmpty) return marks;
  final sorted = [...marks]..sort((a, b) {
    if (a.type != b.type) return a.type.compareTo(b.type);
    return a.start.compareTo(b.start);
  });
  final out = <Mark>[];
  for (final m in sorted) {
    if (m.end <= m.start) continue;
    if (out.isNotEmpty &&
        out.last.type == m.type &&
        out.last.href == m.href &&
        out.last.end >= m.start) {
      final prev = out.removeLast();
      out.add(Mark(prev.start, m.end > prev.end ? m.end : prev.end, m.type,
          href: m.href));
    } else {
      out.add(m);
    }
  }
  return out;
}

const String _asciiPunct = r'''!"#$%&'()*+,-./:;<=>?@[\]^_`{|}~''';

/// Index of the next [needle] at/after [from] that is not backslash-escaped.
int _indexOfUnescaped(String src, String needle, int from) {
  var j = from;
  while (true) {
    j = src.indexOf(needle, j);
    if (j < 0) return -1;
    if (j == 0 || src[j - 1] != r'\') return j;
    j += 1;
  }
}

/// Single-line link reference definition ` [label]: dest "title"`.
({String label, String dest, String? title})? parseRefDefinition(String raw) {
  final lead = raw.length - raw.trimLeft().length;
  if (lead > 3) return null;
  final t = raw.trim();
  if (!t.startsWith('[')) return null;
  final close = matchingBracket(t, 0);
  if (close < 0 || close + 1 >= t.length || t[close + 1] != ':') return null;
  final label = t.substring(1, close);
  if (label.trim().isEmpty) return null;
  // Labels may not contain unescaped brackets.
  for (var k = 1; k < close; k++) {
    if ((t[k] == '[' || t[k] == ']') && t[k - 1] != r'\') return null;
  }
  final rest = t.substring(close + 2).trim();
  if (rest.isEmpty) return null;
  // Reuse the suffix parser by appending a virtual `)` and requiring full
  // consumption.
  final suffix = parseLinkSuffix('$rest)', 0);
  if (suffix == null || suffix.next != rest.length + 1) return null;
  return (label: label, dest: suffix.dest, title: suffix.title);
}

/// Case-fold and collapse internal whitespace (spec label matching).
String normalizeLabel(String label) =>
    label.trim().split(RegExp(r'\s+')).join(' ').toLowerCase();

/// The `]` matching the `[` at [open], honoring nesting and escapes; -1 if
/// none.
int matchingBracket(String src, int open) {
  var depth = 0;
  var j = open;
  while (j < src.length) {
    final c = src[j];
    if (c == r'\' && j + 1 < src.length) {
      j += 2;
      continue;
    }
    if (c == '[') depth++;
    if (c == ']') {
      depth--;
      if (depth == 0) return j;
    }
    j++;
  }
  return -1;
}

/// GFM extended autolink starting at src[i] (caller checks the word
/// boundary): bare http(s)/ftp URLs, `www.` (href gains http://), bare
/// emails. Returns (consumed chars, href), or null.
(int, String)? extendedAutolink(String src, int i) {
  final rest = src.substring(i);
  final lower = rest.toLowerCase();
  for (final (prefix, implied) in [
    ('https://', ''),
    ('http://', ''),
    ('ftp://', ''),
    ('www.', 'http://'),
  ]) {
    if (lower.startsWith(prefix)) {
      var end = rest.length;
      for (var k = 0; k < rest.length; k++) {
        final c = rest[k];
        if (c == ' ' || c == '\t' || c == '\n' || c == '<') {
          end = k;
          break;
        }
      }
      end = _trimAutolinkEnd(rest.substring(0, end));
      final candidate = rest.substring(0, end);
      final afterScheme =
          implied.isEmpty ? candidate.substring(prefix.length) : candidate;
      final domain = afterScheme.split(RegExp(r'[/?#]')).first;
      if (!_validAutolinkDomain(domain)) return null;
      if (candidate.isEmpty) return null;
      return (candidate.length, '$implied$candidate');
    }
  }
  // Bare email.
  if (RegExp(r'^[A-Za-z0-9]').hasMatch(rest)) {
    final m = RegExp(r'^[A-Za-z0-9._+-]+@[A-Za-z0-9._-]+').firstMatch(rest);
    if (m != null) {
      var text = m.group(0)!;
      while (text.endsWith('.')) {
        text = text.substring(0, text.length - 1);
      }
      final at = text.indexOf('@');
      final domain = at >= 0 ? text.substring(at + 1) : '';
      if (at > 0 &&
          domain.contains('.') &&
          RegExp(r'[A-Za-z0-9]$').hasMatch(text)) {
        return (text.length, 'mailto:$text');
      }
    }
  }
  return null;
}

int _trimAutolinkEnd(String s) {
  var end = s.length;
  while (end > 0) {
    final kept = s.substring(0, end);
    final last = kept[kept.length - 1];
    if ('?!.,:*_~\'"'.contains(last)) {
      end -= 1;
      continue;
    }
    if (last == ')') {
      final opens = '('.allMatches(kept).length;
      final closes = ')'.allMatches(kept).length;
      if (closes > opens) {
        end -= 1;
        continue;
      }
      return end;
    }
    if (last == ';') {
      final amp = kept.lastIndexOf('&');
      if (amp >= 0) {
        final body = kept.substring(amp + 1, end - 1);
        if (body.isNotEmpty &&
            RegExp(r'^[A-Za-z0-9]+$').hasMatch(body)) {
          end = amp;
          continue;
        }
      }
      return end;
    }
    return end;
  }
  return 0;
}

bool _validAutolinkDomain(String domain) {
  if (domain.isEmpty ||
      !RegExp(r'^[A-Za-z0-9._-]+$').hasMatch(domain)) {
    return false;
  }
  final segments = domain.split('.');
  if (segments.length < 2 || segments.any((s) => s.isEmpty)) return false;
  final lastTwo = segments.length >= 2
      ? segments.sublist(segments.length - 2)
      : segments;
  return !lastTwo.any((s) => s.contains('_'));
}

/// Inline raw HTML at src[i] == '<': open tag, closing tag, comment,
/// processing instruction, declaration or CDATA. Returns the exclusive end
/// index, or -1. Mirrors the Rust engine's inline_html_end.
int inlineHtmlEnd(String src, int i) {
  if (i >= src.length || src[i] != '<') return -1;
  int findPat(int from, String pat) => src.indexOf(pat, from);
  // Comment (<!--> and <!---> count as empty comments).
  if (src.startsWith('!--', i + 1)) {
    if (src.startsWith('>', i + 4)) return i + 5;
    if (src.startsWith('->', i + 4)) return i + 6;
    final p = findPat(i + 4, '-->');
    return p < 0 ? -1 : p + 3;
  }
  if (src.startsWith('![CDATA[', i + 1)) {
    final p = findPat(i + 9, ']]>');
    return p < 0 ? -1 : p + 3;
  }
  if (i + 1 < src.length && src[i + 1] == '?') {
    final p = findPat(i + 2, '?>');
    return p < 0 ? -1 : p + 2;
  }
  bool isAlpha(String c) =>
      (c.codeUnitAt(0) | 0x20) >= 0x61 && (c.codeUnitAt(0) | 0x20) <= 0x7A;
  bool isAlnum(String c) => isAlpha(c) || (c.codeUnitAt(0) >= 0x30 && c.codeUnitAt(0) <= 0x39);
  if (i + 1 < src.length && src[i + 1] == '!') {
    if (i + 2 >= src.length || !isAlpha(src[i + 2])) return -1;
    final p = findPat(i + 2, '>');
    return p < 0 ? -1 : p + 1;
  }
  var closing = false;
  var p = i + 1;
  if (p < src.length && src[p] == '/') {
    closing = true;
    p++;
  }
  if (p >= src.length || !isAlpha(src[p])) return -1;
  while (p < src.length && (isAlnum(src[p]) || src[p] == '-')) {
    p++;
  }
  int skipWs(int q) {
    while (q < src.length && (src[q] == ' ' || src[q] == '\t' || src[q] == '\n')) {
      q++;
    }
    return q;
  }
  if (closing) {
    final q = skipWs(p);
    return (q < src.length && src[q] == '>') ? q + 1 : -1;
  }
  while (true) {
    final q = skipWs(p);
    if (q < src.length && src[q] == '>') return q + 1;
    if (q + 1 < src.length && src[q] == '/' && src[q + 1] == '>') return q + 2;
    if (q == p) return -1; // an attribute needs whitespace before it
    if (q >= src.length) return -1;
    final c0 = src[q];
    if (!(isAlpha(c0) || c0 == '_' || c0 == ':')) return -1;
    var r = q + 1;
    while (r < src.length &&
        (isAlnum(src[r]) || src[r] == '_' || src[r] == '.' || src[r] == ':' || src[r] == '-')) {
      r++;
    }
    final eq = skipWs(r);
    if (eq < src.length && src[eq] == '=') {
      final v = skipWs(eq + 1);
      if (v >= src.length) return -1;
      final quote = src[v];
      if (quote == '"' || quote == "'") {
        var w = v + 1;
        while (w < src.length && src[w] != quote) {
          w++;
        }
        if (w >= src.length) return -1;
        p = w + 1;
      } else {
        var w = v;
        while (w < src.length &&
            !' \t\n"\'=<>`'.contains(src[w])) {
          w++;
        }
        if (w == v) return -1;
        p = w;
      }
    } else {
      p = r;
    }
  }
}

/// Curated named-entity table (the spec set plus common real-world names);
/// unknown entities stay literal, which is exactly the spec behavior.
const Map<String, String> _namedEntities = {
  'AElig': 'Æ',
  'Alpha': 'Α',
  'Beta': 'Β',
  'ClockwiseContourIntegral': '∲',
  'Dagger': '‡',
  'Dcaron': 'Ď',
  'Delta': 'Δ',
  'DifferentialD': 'ⅆ',
  'Gamma': 'Γ',
  'HilbertSpace': 'ℋ',
  'Lambda': 'Λ',
  'OElig': 'Œ',
  'Omega': 'Ω',
  'Phi': 'Φ',
  'Pi': 'Π',
  'Psi': 'Ψ',
  'Scaron': 'Š',
  'Sigma': 'Σ',
  'Theta': 'Θ',
  'Yuml': 'Ÿ',
  'aacute': 'á',
  'acirc': 'â',
  'acute': '´',
  'agrave': 'à',
  'alpha': 'α',
  'amp': '&',
  'apos': '\'',
  'aring': 'å',
  'asymp': '≈',
  'atilde': 'ã',
  'auml': 'ä',
  'beta': 'β',
  'brvbar': '¦',
  'bull': '•',
  'cap': '∩',
  'ccedil': 'ç',
  'cedil': '¸',
  'cent': '¢',
  'chi': 'χ',
  'circ': 'ˆ',
  'clubs': '♣',
  'copy': '©',
  'cup': '∪',
  'curren': '¤',
  'dagger': '†',
  'darr': '↓',
  'deg': '°',
  'delta': 'δ',
  'diams': '♦',
  'divide': '÷',
  'eacute': 'é',
  'ecirc': 'ê',
  'egrave': 'è',
  'empty': '∅',
  'emsp': ' ',
  'ensp': ' ',
  'epsilon': 'ε',
  'equiv': '≡',
  'eta': 'η',
  'euml': 'ë',
  'euro': '€',
  'exist': '∃',
  'fnof': 'ƒ',
  'forall': '∀',
  'frac12': '½',
  'frac14': '¼',
  'frac34': '¾',
  'frasl': '⁄',
  'gamma': 'γ',
  'ge': '≥',
  'gt': '>',
  'harr': '↔',
  'hearts': '♥',
  'hellip': '…',
  'iacute': 'í',
  'icirc': 'î',
  'iexcl': '¡',
  'infin': '∞',
  'int': '∫',
  'iota': 'ι',
  'iquest': '¿',
  'isin': '∈',
  'iuml': 'ï',
  'kappa': 'κ',
  'lambda': 'λ',
  'lang': '⟨',
  'laquo': '«',
  'larr': '←',
  'lceil': '⌈',
  'ldquo': '“',
  'le': '≤',
  'lfloor': '⌊',
  'loz': '◊',
  'lsquo': '‘',
  'lt': '<',
  'macr': '¯',
  'mdash': '—',
  'micro': 'µ',
  'middot': '·',
  'minus': '−',
  'mu': 'μ',
  'nbsp': ' ',
  'ndash': '–',
  'ne': '≠',
  'ngE': '≧̸',
  'not': '¬',
  'notin': '∉',
  'ntilde': 'ñ',
  'nu': 'ν',
  'oacute': 'ó',
  'ocirc': 'ô',
  'oelig': 'œ',
  'omega': 'ω',
  'oplus': '⊕',
  'ordf': 'ª',
  'ordm': 'º',
  'oslash': 'ø',
  'otilde': 'õ',
  'otimes': '⊗',
  'ouml': 'ö',
  'para': '¶',
  'permil': '‰',
  'perp': '⊥',
  'phi': 'φ',
  'pi': 'π',
  'plusmn': '±',
  'pound': '£',
  'prod': '∏',
  'psi': 'ψ',
  'quot': '"',
  'radic': '√',
  'rang': '⟩',
  'raquo': '»',
  'rarr': '→',
  'rceil': '⌉',
  'rdquo': '”',
  'reg': '®',
  'rfloor': '⌋',
  'rho': 'ρ',
  'rsquo': '’',
  'scaron': 'š',
  'sdot': '⋅',
  'sect': '§',
  'shy': '­',
  'sigma': 'σ',
  'spades': '♠',
  'sub': '⊂',
  'sum': '∑',
  'sup': '⊃',
  'sup1': '¹',
  'sup2': '²',
  'sup3': '³',
  'szlig': 'ß',
  'tau': 'τ',
  'theta': 'θ',
  'thinsp': ' ',
  'tilde': '˜',
  'times': '×',
  'trade': '™',
  'uacute': 'ú',
  'uarr': '↑',
  'ucirc': 'û',
  'uml': '¨',
  'upsilon': 'υ',
  'uuml': 'ü',
  'xi': 'ξ',
  'yen': '¥',
  'zeta': 'ζ',
  'zwj': '‍',
  'zwnj': '‌',
};

/// Parse an entity/numeric character reference starting at `&` (src[i]).
/// Returns (decoded string, chars consumed), or null.
(String, int)? parseEntity(String src, int i) {
  if (i >= src.length || src[i] != '&') return null;
  final semi = src.indexOf(';', i + 1);
  if (semi < 0 || semi - i > 33) return null;
  final body = src.substring(i + 1, semi);
  if (body.startsWith('#')) {
    var digits = body.substring(1);
    var radix = 10;
    if (digits.startsWith('x') || digits.startsWith('X')) {
      digits = digits.substring(1);
      radix = 16;
    }
    if (digits.isEmpty || digits.length > 7) return null;
    final n = int.tryParse(digits, radix: radix);
    if (n == null) return null;
    final code = (n == 0 || n > 0x10FFFF || (n >= 0xD800 && n <= 0xDFFF)) ? 0xFFFD : n;
    return (String.fromCharCode(code), semi - i + 1);
  }
  final v = _namedEntities[body];
  return v == null ? null : (v, semi - i + 1);
}

/// Unescape backslash-escaped ASCII punctuation (public mirror).
String unescapeMd(String s) => _unescapeMd(s);

/// Unescape backslash-escaped ASCII punctuation.
String _unescapeMd(String s) {
  final out = StringBuffer();
  var i = 0;
  while (i < s.length) {
    if (s[i] == r'\' && i + 1 < s.length && _asciiPunct.contains(s[i + 1])) {
      out.write(s[i + 1]);
      i += 2;
    } else if (s[i] == '&') {
      // Entity references decode inside destinations/titles/info strings.
      final ent = parseEntity(s, i);
      if (ent != null) {
        out.write(ent.$1);
        i += ent.$2;
      } else {
        out.write(s[i]);
        i++;
      }
    } else {
      out.write(s[i]);
      i++;
    }
  }
  return out.toString();
}

/// Inline-link suffix `(dest "title")` parsed right AFTER the `(`. Returns
/// dest/title and the index just past the closing `)`. Mirrors the Rust
/// engine.
({String dest, String? title, int next})? parseLinkSuffix(String src, int i) {
  final n = src.length;
  bool ws(String c) => c == ' ' || c == '\t' || c == '\n';
  while (i < n && ws(src[i])) {
    i++;
  }
  String dest;
  if (i < n && src[i] == '<') {
    var j = i + 1;
    while (j < n && src[j] != '>' && src[j] != '\n' && src[j] != '<') {
      j++;
    }
    if (j >= n || src[j] != '>') return null;
    dest = _unescapeMd(src.substring(i + 1, j));
    i = j + 1;
  } else {
    var depth = 0;
    final start = i;
    while (i < n) {
      final c = src[i];
      if (ws(c)) break;
      if (c == r'\' && i + 1 < n) {
        i += 2;
        continue;
      }
      if (c == '(') depth++;
      if (c == ')') {
        if (depth == 0) break;
        depth--;
      }
      i++;
    }
    if (depth != 0) return null;
    dest = _unescapeMd(src.substring(start, i));
  }
  while (i < n && ws(src[i])) {
    i++;
  }
  String? title;
  if (i < n && (src[i] == '"' || src[i] == "'" || src[i] == '(')) {
    final close = src[i] == '(' ? ')' : src[i];
    var j = i + 1;
    final buf = StringBuffer();
    while (j < n && src[j] != close) {
      if (src[j] == r'\' && j + 1 < n) {
        buf.write(src[j + 1]);
        j += 2;
        continue;
      }
      buf.write(src[j]);
      j++;
    }
    if (j >= n) return null;
    title = buf.toString();
    i = j + 1;
    while (i < n && ws(src[i])) {
      i++;
    }
  }
  if (i < n && src[i] == ')') {
    return (dest: dest, title: title, next: i + 1);
  }
  return null;
}

/// CommonMark autolink target for `<inner>`: absolute URI → itself, bare
/// email → mailto:, else null. Mirrors the Rust engine.
String? autolinkTarget(String inner) {
  if (inner.isEmpty || inner.contains(RegExp(r'[\s<]'))) return null;
  if (RegExp(r'^[A-Za-z][A-Za-z0-9+.-]{1,31}:.').hasMatch(inner)) return inner;
  if (!inner.contains(r'\') &&
      RegExp(r'^[^@\s]+@[^@\s]+\.[^@\s.]+$').hasMatch(inner)) {
    return 'mailto:$inner';
  }
  return null;
}

/// Parse inline Markdown (`**b**`, `*i*`/`_i_`, `` `c` ``, `~~s~~`,
/// `[t](url)`, `\*escapes\*`, `<autolinks>`) into clean text plus marks.
({String text, List<Mark> marks}) parseInline(
  String src, {
  Map<String, ({String dest, String? title})> defs = const {},
}) {
  final out = StringBuffer();
  final marks = <Mark>[];
  final delims = <_Delim>[];
  var i = 0;

  void addInner(String inner, String type, {String? href}) {
    final start = out.length;
    final parsed = parseInline(inner, defs: defs);
    out.write(parsed.text);
    for (final m in parsed.marks) {
      marks.add(m.shifted(start));
    }
    marks.add(Mark(start, out.length, type, href: href));
  }

  void addLink(String kind, String label, String href, String? title) {
    final start = out.length;
    final parsed = parseInline(label, defs: defs);
    out.write(parsed.text);
    for (final m in parsed.marks) {
      marks.add(m.shifted(start));
    }
    marks.add(Mark(start, out.length, kind, href: href, title: title));
  }

  while (i < src.length) {
    // Backslash escape: `\*` is a literal `*` (any ASCII punctuation).
    if (src[i] == r'\' &&
        i + 1 < src.length &&
        _asciiPunct.contains(src[i + 1])) {
      out.write(src[i + 1]);
      i += 2;
      continue;
    }
    // Entity / numeric character reference: decodes to plain TEXT (the
    // result can't open emphasis or any structure).
    if (src[i] == '&') {
      final ent = parseEntity(src, i);
      if (ent != null) {
        out.write(ent.$1);
        i += ent.$2;
        continue;
      }
    }
    // Autolink: <https://…> / <user@host> — the URL is both text and target.
    if (src[i] == '<') {
      final close = src.indexOf('>', i + 1);
      if (close > i) {
        final inner = src.substring(i + 1, close);
        final href = autolinkTarget(inner);
        if (href != null) {
          final start = out.length;
          out.write(inner);
          marks.add(Mark(start, out.length, 'link', href: href));
          i = close + 1;
          continue;
        }
      }
      // Raw inline HTML: a valid tag/comment/PI/declaration/CDATA shape
      // passes through verbatim under an `html` mark.
      final htmlEnd = inlineHtmlEnd(src, i);
      if (htmlEnd > i) {
        final raw = src.substring(i, htmlEnd);
        final start = out.length;
        out.write(raw);
        marks.add(Mark(start, out.length, 'html'));
        i = htmlEnd;
        continue;
      }
    }
    // Image: ![alt](dest "title") | ![alt][label] | ![alt][] | ![alt] —
    // same bridge as links; the alt keeps its inner marks (HTML flattens
    // them to plain text, markdown re-renders them).
    if (src[i] == '!' && i + 1 < src.length && src[i + 1] == '[') {
      final close = matchingBracket(src, i + 1);
      if (close > i + 2) {
        final label = src.substring(i + 2, close);
        if (close + 1 < src.length && src[close + 1] == '(') {
          final suffix = parseLinkSuffix(src, close + 2);
          if (suffix != null) {
            addLink('image', label, suffix.dest, suffix.title);
            i = suffix.next;
            continue;
          }
        }
        if (defs.isNotEmpty) {
          var refLabel = label;
          var next = close + 1;
          if (close + 1 < src.length && src[close + 1] == '[') {
            final end2 = matchingBracket(src, close + 1);
            if (end2 > close + 1) {
              final second = src.substring(close + 2, end2);
              if (second.isNotEmpty) refLabel = second;
              next = end2 + 1;
            }
          }
          final def = defs[normalizeLabel(refLabel)];
          if (def != null) {
            addLink('image', label, def.dest, def.title);
            i = next;
            continue;
          }
        }
      }
    }
    // Links: [text](dest "title") | [text][label] | [text][] | [shortcut]
    if (src[i] == '[') {
      final close = matchingBracket(src, i);
      if (close > i + 1) {
        final label = src.substring(i + 1, close);
        if (close + 1 < src.length && src[close + 1] == '(') {
          final suffix = parseLinkSuffix(src, close + 2);
          if (suffix != null) {
            addLink('link', label, suffix.dest, suffix.title);
            i = suffix.next;
            continue;
          }
        }
        if (defs.isNotEmpty) {
          var refLabel = label;
          var next = close + 1;
          if (close + 1 < src.length && src[close + 1] == '[') {
            final end2 = matchingBracket(src, close + 1);
            if (end2 > close + 1) {
              final second = src.substring(close + 2, end2);
              if (second.isNotEmpty) refLabel = second;
              next = end2 + 1;
            }
          }
          final def = defs[normalizeLabel(refLabel)];
          if (def != null) {
            addLink('link', label, def.dest, def.title);
            i = next;
            continue;
          }
        }
      }
    }
    // Emphasis delimiters (* and _): record the run with flanking; pairing
    // happens after the scan (spec algorithm, mirrors the Rust engine).
    if (src[i] == '*' || src[i] == '_') {
      final c = src[i];
      var j = i;
      while (j < src.length && src[j] == c) {
        j++;
      }
      final count = j - i;
      final f = _flanking(c, i == 0 ? null : src[i - 1],
          j < src.length ? src[j] : null);
      delims.add(_Delim(c, out.length, count, f.open, f.close));
      out.write(c * count);
      i = j;
      continue;
    }
    // GFM extended autolinks: bare www./http(s)/ftp URLs and emails at a
    // word boundary (start, whitespace, or `*`/`_`/`~`/`(`).
    if (i == 0 ||
        src[i - 1] == ' ' ||
        src[i - 1] == '\t' ||
        src[i - 1] == '\n' ||
        '*_~('.contains(src[i - 1])) {
      final auto = extendedAutolink(src, i);
      if (auto != null) {
        final text = src.substring(i, i + auto.$1);
        final start = out.length;
        out.write(text);
        marks.add(Mark(start, out.length, 'link', href: auto.$2));
        i += auto.$1;
        continue;
      }
    }
    // GFM strikethrough: one or two tildes close on a run of the same
    // length (three or more stay literal).
    if (src[i] == '~') {
      var n = 0;
      while (i + n < src.length && src[i + n] == '~') {
        n++;
      }
      if (n <= 2) {
        var j = i + n;
        var close = -1;
        while (j < src.length) {
          if (src[j] == '~' && (j == 0 || src[j - 1] != r'\')) {
            var m = 0;
            while (j + m < src.length && src[j + m] == '~') {
              m++;
            }
            if (m == n && j > i + n) {
              close = j;
              break;
            }
            j += m;
          } else {
            j++;
          }
        }
        if (close >= 0) {
          addInner(src.substring(i + n, close), 'strike');
          i = close + n;
          continue;
        }
      }
      out.write('~' * n);
      i += n;
      continue;
    }
    // Inline code: an N-backtick run closes only on a run of exactly N;
    // line endings become spaces; one leading+trailing space strips when
    // both exist and the content isn't all spaces. No escapes inside.
    if (src[i] == '`') {
      var n = 0;
      while (i + n < src.length && src[i + n] == '`') {
        n++;
      }
      var j = i + n;
      var close = -1;
      while (j < src.length) {
        if (src[j] == '`') {
          var m = 0;
          while (j + m < src.length && src[j + m] == '`') {
            m++;
          }
          if (m == n) {
            close = j;
            break;
          }
          j += m;
        } else {
          j++;
        }
      }
      if (close >= 0) {
        var inner = src.substring(i + n, close).replaceAll('\n', ' ');
        if (inner.length >= 2 &&
            inner.startsWith(' ') &&
            inner.endsWith(' ') &&
            inner.trim().isNotEmpty) {
          inner = inner.substring(1, inner.length - 1);
        }
        final start = out.length;
        out.write(inner);
        marks.add(Mark(start, out.length, 'code'));
        i = close + n;
        continue;
      }
      // No closer: the run is literal text.
      out.write('`' * n);
      i += n;
      continue;
    }
    out.write(src[i]);
    i++;
  }
  final processed = _processEmphasis(out.toString(), marks, delims);
  // NOTE: parse output is intentionally NOT merge-normalized — nested
  // same-type emphasis (`<em><em>`) must survive for spec fidelity; the
  // editor's own operations (applyMark etc.) still normalize.
  marks.sort((a, b) => a.start != b.start ? a.start - b.start : a.end - b.end);
  return (text: processed, marks: marks);
}

/// One run of `*`/`_` delimiters, tracked in output coordinates.
class _Delim {
  _Delim(this.c, this.start, this.count, this.canOpen, this.canClose)
      : curStart = start,
        orig = count;
  final String c;
  final int start;
  int curStart;
  int count;
  final int orig;
  final bool canOpen;
  final bool canClose;
}

bool _isMdPunct(String? c) {
  if (c == null) return false;
  return _asciiPunct.contains(c) ||
      (c.codeUnitAt(0) >= 0x2010 && c.codeUnitAt(0) <= 0x2027);
}

({bool open, bool close}) _flanking(String c, String? prev, String? next) {
  final prevWs = prev == null || prev.trim().isEmpty;
  final nextWs = next == null || next.trim().isEmpty;
  final prevPunct = _isMdPunct(prev);
  final nextPunct = _isMdPunct(next);
  final left = !nextWs && (!nextPunct || prevWs || prevPunct);
  final right = !prevWs && (!prevPunct || nextWs || nextPunct);
  if (c == '_') {
    return (
      open: left && (!right || prevPunct),
      close: right && (!left || nextPunct),
    );
  }
  return (open: left, close: right);
}

/// Spec process-emphasis: pair closers with the nearest valid opener (same
/// char, rule of 3), strong before em; delete used delimiter characters and
/// remap mark offsets. Returns the rebuilt text.
String _processEmphasis(String text, List<Mark> marks, List<_Delim> delims) {
  if (delims.isEmpty) return text;
  final deletions = <(int, int)>[];
  final pending = <Mark>[];

  var closerI = 0;
  while (closerI < delims.length) {
    final cl = delims[closerI];
    if (cl.count == 0 || !cl.canClose) {
      closerI++;
      continue;
    }
    int? openerI;
    for (var k = closerI - 1; k >= 0; k--) {
      final o = delims[k];
      if (o.count == 0 || !o.canOpen || o.c != cl.c) continue;
      if ((o.canClose || cl.canOpen) &&
          (o.orig + cl.orig) % 3 == 0 &&
          !(o.orig % 3 == 0 && cl.orig % 3 == 0)) {
        continue;
      }
      openerI = k;
      break;
    }
    if (openerI == null) {
      closerI++;
      continue;
    }
    final o = delims[openerI];
    final useN = (o.count >= 2 && cl.count >= 2) ? 2 : 1;
    o.count -= useN;
    final oDel = o.start + o.count;
    deletions.add((oDel, useN));
    final cDel = cl.curStart;
    deletions.add((cDel, useN));
    cl.curStart += useN;
    cl.count -= useN;
    pending.add(Mark(oDel + useN, cDel, useN == 2 ? 'bold' : 'italic'));
    for (var k = openerI + 1; k < closerI; k++) {
      delims[k].count = 0;
    }
    if (cl.count == 0) closerI++;
  }

  if (deletions.isEmpty) return text;
  marks.addAll(pending);
  deletions.sort((a, b) => a.$1.compareTo(b.$1));

  final keep = StringBuffer();
  final removedBefore = List<int>.filled(text.length + 1, 0);
  var di = 0;
  var removed = 0;
  var skipUntil = 0;
  for (var idx = 0; idx < text.length; idx++) {
    removedBefore[idx] = removed;
    if (idx >= skipUntil && di < deletions.length && deletions[di].$1 == idx) {
      skipUntil = idx + deletions[di].$2;
      di++;
    }
    if (idx < skipUntil) {
      removed++;
    } else {
      keep.write(text[idx]);
    }
  }
  removedBefore[text.length] = removed;

  for (var k = 0; k < marks.length; k++) {
    final m = marks[k];
    final ns = m.start - removedBefore[m.start.clamp(0, text.length)];
    final ne = m.end - removedBefore[m.end.clamp(0, text.length)];
    marks[k] = Mark(ns, ne, m.type, href: m.href, title: m.title);
  }
  marks.removeWhere((m) => m.end <= m.start);
  return keep.toString();
}

/// A paragraph whose text LOOKS like a block marker (`- x`, `> x`, `# x`,
/// `1. x`, `---`) must escape its leader or it changes kind on re-parse.
/// Mirrors the Rust engine.
String escapeBlockLeader(String line) {
  final dividerLike =
      line.length >= 3 && line.split('').every((c) => c == '-');
  if (line.startsWith('- ') ||
      line.startsWith('+ ') ||
      line.startsWith('> ') ||
      dividerLike ||
      RegExp(r'^#{1,6} ').hasMatch(line)) {
    return '\\$line';
  }
  final numbered = RegExp(r'^(\d+)\. ').firstMatch(line);
  if (numbered != null) {
    return '${numbered.group(1)}\\. ${line.substring(numbered.end)}';
  }
  return line;
}

/// Escape characters the inline grammar would otherwise interpret.
String escapeInline(String text) {
  final out = StringBuffer();
  for (var i = 0; i < text.length; i++) {
    final c = text[i];
    // A backslash right before a newline IS a hard break — keep it raw.
    if (c == r'\' && i + 1 < text.length && text[i + 1] == '\n') {
      out.write(c);
      continue;
    }
    if (r'\*_`~[]<'.contains(c)) out.write(r'\');
    out.write(c);
  }
  return out.toString();
}

/// Serialize text + marks back to inline Markdown (copy/export). Properly
/// NESTED rendering — the outermost mark opens once over its whole range,
/// inner marks recurse inside (`**bold *italic* tail**`); literal segments
/// are escaped; code spans stay raw; a link whose text is its own target
/// becomes an `<autolink>`. Mirrors the Rust engine's render_span.
String inlineToMarkdown(String text, List<Mark> marks) {
  if (marks.isEmpty) return text;
  return _renderSpan(text, 0, text.length, marks);
}

String _renderSpan(String text, int lo, int hi, List<Mark> marks) {
  final out = StringBuffer();
  var pos = lo;
  while (pos < hi) {
    // Next mark by clipped start; ties prefer the widest (outermost).
    Mark? pick;
    var ps = 0, pe = 0;
    for (final m in marks) {
      final s = m.start < pos ? pos : m.start;
      final e = m.end > hi ? hi : m.end;
      if (e <= s) continue;
      if (pick == null || s < ps || (s == ps && e > pe)) {
        pick = m;
        ps = s;
        pe = e;
      }
    }
    if (pick == null) {
      out.write(escapeInline(text.substring(pos, hi)));
      break;
    }
    var lead = escapeInline(text.substring(pos, ps));
    if (pick.type == 'link' && lead.endsWith('!')) {
      lead = '${lead.substring(0, lead.length - 1)}\\!';
    }
    out.write(lead);
    if (pick.type == 'html') {
      // Raw inline HTML writes back verbatim.
      out.write(text.substring(ps, pe));
      pos = pe;
      continue;
    }
    if (pick.type == 'code') {
      // Code spans are literal — no escaping, no nested marks. The fence is
      // one backtick longer than any run inside; a space pads content that
      // starts/ends with a backtick or with stripped-on-read spaces.
      final raw = text.substring(ps, pe);
      var longest = 0, cur = 0;
      for (var k = 0; k < raw.length; k++) {
        if (raw[k] == '`') {
          cur++;
          if (cur > longest) longest = cur;
        } else {
          cur = 0;
        }
      }
      final fence = '`' * (longest + 1);
      final pad = raw.startsWith('`') ||
          raw.endsWith('`') ||
          (raw.startsWith(' ') && raw.endsWith(' ') && raw.trim().isNotEmpty);
      out.write(pad ? '$fence $raw $fence' : '$fence$raw$fence');
      pos = pe;
      continue;
    }
    final inner = [
      for (final m in marks)
        if (!identical(m, pick) && m.end > ps && m.start < pe) m,
    ];
    final body = _renderSpan(text, ps, pe, inner);
    switch (pick.type) {
      case 'bold':
        out.write('**$body**');
      case 'italic':
        out.write('*$body*');
      case 'strike':
        out.write('~~$body~~');
      case 'link':
        final href = pick.href ?? '';
        final plain = text.substring(ps, pe);
        if (pick.title == null &&
            plain.startsWith('www.') &&
            href == 'http://$plain') {
          // A bare `www.` link writes back bare (GFM re-links it on read).
          out.write(plain);
        } else if (pick.title == null &&
            (href == plain || href == 'mailto:$plain')) {
          out.write('<$plain>');
        } else {
          final dest = href.contains(RegExp(r'\s')) ? '<$href>' : href;
          final t = pick.title;
          out.write(
            t == null
                ? '[$body]($dest)'
                : '[$body]($dest "${t.replaceAll('"', r'\"')}")',
          );
        }
      case 'image':
        final href = pick.href ?? '';
        final dest = href.contains(RegExp(r'\s')) ? '<$href>' : href;
        final t = pick.title;
        out.write(
          t == null
              ? '![$body]($dest)'
              : '![$body]($dest "${t.replaceAll('"', r'\"')}")',
        );
      default:
        out.write(body);
    }
    pos = pe;
  }
  return out.toString();
}
