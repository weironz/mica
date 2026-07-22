/// Structured page properties over the document root's raw front-matter string.
///
/// **Dart mirror of the Rust authority** `crates/markdown/src/properties.rs`.
/// The Rust side is authoritative (CLAUDE.md #2); this must stay byte-for-byte
/// behaviourally identical — parse/upsert/remove here and there have to agree,
/// or a property edited on desktop vs re-read on web would diverge. When you
/// change one side, change the other and keep both test suites green.
///
/// Front matter stays the SOLE authority: it lives verbatim as the raw inner
/// string on the document root block's `data['front_matter']`. These functions
/// are a lazy view + surgical edit over that string — no second representation.
/// See docs/page-properties.md for the round-trip-invariant relaxation
/// (byte-exact → normalized-subset) this enables.
library;

/// A typed front-matter value — the small closed set inferred from a YAML scalar
/// (Text / Number / Checkbox / Date / List). `tags` is just a [PropList].
sealed class PropertyValue {
  const PropertyValue();

  /// JSON tag matching the Rust `#[serde(tag = "type", content = "value")]`
  /// shape, so a value can cross the FFI / be stored identically on both sides.
  Map<String, dynamic> toJson();

  static PropertyValue fromJson(Map<String, dynamic> json) {
    final value = json['value'];
    switch (json['type']) {
      case 'text':
        return PropText(value as String);
      case 'number':
        return PropNumber((value as num).toDouble());
      case 'checkbox':
        return PropCheckbox(value as bool);
      case 'date':
        return PropDate(value as String);
      case 'list':
        return PropList(List<String>.from(value as List));
      default:
        return const PropText('');
    }
  }
}

class PropText extends PropertyValue {
  const PropText(this.value);
  final String value;
  @override
  Map<String, dynamic> toJson() => {'type': 'text', 'value': value};
  @override
  bool operator ==(Object other) => other is PropText && other.value == value;
  @override
  int get hashCode => value.hashCode;
}

class PropNumber extends PropertyValue {
  const PropNumber(this.value);
  final double value;
  @override
  Map<String, dynamic> toJson() => {'type': 'number', 'value': value};
  @override
  bool operator ==(Object other) => other is PropNumber && other.value == value;
  @override
  int get hashCode => value.hashCode;
}

class PropCheckbox extends PropertyValue {
  const PropCheckbox(this.value);
  final bool value;
  @override
  Map<String, dynamic> toJson() => {'type': 'checkbox', 'value': value};
  @override
  bool operator ==(Object other) => other is PropCheckbox && other.value == value;
  @override
  int get hashCode => value.hashCode;
}

class PropDate extends PropertyValue {
  const PropDate(this.value);
  final String value;
  @override
  Map<String, dynamic> toJson() => {'type': 'date', 'value': value};
  @override
  bool operator ==(Object other) => other is PropDate && other.value == value;
  @override
  int get hashCode => value.hashCode;
}

class PropList extends PropertyValue {
  const PropList(this.items);
  final List<String> items;
  @override
  Map<String, dynamic> toJson() => {'type': 'list', 'value': items};
  @override
  bool operator ==(Object other) =>
      other is PropList &&
      other.items.length == items.length &&
      _listEq(other.items, items);
  @override
  int get hashCode => Object.hashAll(items);
}

bool _listEq(List<String> a, List<String> b) {
  for (var i = 0; i < a.length; i++) {
    if (a[i] != b[i]) return false;
  }
  return true;
}

/// One top-level front-matter key and its typed value.
class Property {
  const Property(this.key, this.value);
  final String key;
  final PropertyValue value;
  @override
  bool operator ==(Object other) =>
      other is Property && other.key == key && other.value == value;
  @override
  int get hashCode => Object.hash(key, value);
  @override
  String toString() => 'Property($key, $value)';
}

/// Parse the flat, editable subset of a front-matter string into typed
/// properties, in source order. `frontMatter` is the RAW INNER text (no `---`
/// fences). Mirrors `parse_properties` in the Rust authority.
List<Property> parseProperties(String frontMatter) {
  final lines = frontMatter.split('\n');
  final out = <Property>[];
  var i = 0;
  while (i < lines.length) {
    final raw = lines[i];
    if (raw.startsWith(' ') || raw.startsWith('\t')) {
      i += 1;
      continue;
    }
    final trimmed = raw.trim();
    if (trimmed.isEmpty || trimmed.startsWith('#')) {
      i += 1;
      continue;
    }
    final split = _splitKey(raw);
    if (split == null) {
      i += 1;
      continue;
    }
    final key = split.$1;
    final valueText = split.$2.trim();
    if (valueText.isEmpty) {
      // Bare `key:` — scan the indented continuation block.
      final items = <String>[];
      var allListItems = true;
      var sawIndented = false;
      var j = i + 1;
      while (j < lines.length) {
        final cont = lines[j];
        if (!(cont.startsWith(' ') || cont.startsWith('\t'))) break;
        final ct = cont.trim();
        if (ct.isEmpty) break;
        sawIndented = true;
        if (ct.startsWith('-')) {
          items.add(_unquoteScalar(ct.substring(1).trim()));
        } else {
          allListItems = false;
        }
        j += 1;
      }
      if (sawIndented) {
        if (allListItems) {
          out.add(Property(key, PropList(items)));
        }
        // else: nested/complex map — unsurfaced, preserved as bytes.
        i = j;
        continue;
      }
      out.add(Property(key, const PropText('')));
      i += 1;
      continue;
    }
    out.add(Property(key, _inferScalar(valueText)));
    i += 1;
  }
  return out;
}

/// Infer a typed value from a user's raw single-line input (what the property
/// editor commits): empty → empty text, otherwise the same bool/number/date/
/// list/text inference `parseProperties` uses. Mirrors `infer_value`.
PropertyValue inferValue(String raw) {
  final t = raw.trim();
  if (t.isEmpty) return const PropText('');
  return _inferScalar(t);
}

/// Insert or replace `key`'s value, editing only that key's line(s) and leaving
/// the rest byte-exact. New key is appended. Mirrors `upsert_property`.
String upsertProperty(String frontMatter, String key, PropertyValue value) {
  final lines = frontMatter.split('\n');
  final span = _keySpan(lines, key);
  if (span != null) {
    final wasBlockList = span.$2 > span.$1 + 1;
    final rendered = _renderProperty(key, value, wasBlockList);
    final newLines = <String>[
      ...lines.sublist(0, span.$1),
      ...rendered,
      ...lines.sublist(span.$2),
    ];
    return newLines.join('\n');
  }
  final rendered = _renderProperty(key, value, false).join('\n');
  if (frontMatter.isEmpty) return rendered;
  if (frontMatter.endsWith('\n')) return '$frontMatter$rendered';
  return '$frontMatter\n$rendered';
}

/// Remove `key` and its value lines, leaving the rest byte-exact. Unknown key →
/// unchanged. Mirrors `remove_property`.
String removeProperty(String frontMatter, String key) {
  final lines = frontMatter.split('\n');
  final span = _keySpan(lines, key);
  if (span == null) return frontMatter;
  final kept = <String>[
    ...lines.sublist(0, span.$1),
    ...lines.sublist(span.$2),
  ];
  return kept.join('\n');
}

/// `(start, end)` line range a top-level `key` owns (its line + indented block).
(int, int)? _keySpan(List<String> lines, String key) {
  for (var idx = 0; idx < lines.length; idx++) {
    final raw = lines[idx];
    if (raw.startsWith(' ') || raw.startsWith('\t')) continue;
    final split = _splitKey(raw);
    if (split != null && split.$1 == key) {
      var end = idx + 1;
      while (end < lines.length &&
          (lines[end].startsWith(' ') || lines[end].startsWith('\t')) &&
          lines[end].trim().isNotEmpty) {
        end += 1;
      }
      return (idx, end);
    }
  }
  return null;
}

List<String> _renderProperty(String key, PropertyValue value, bool blockList) {
  switch (value) {
    case PropText(:final value):
      return ['$key: ${_quoteIfNeeded(value)}'];
    case PropNumber(:final value):
      return ['$key: ${_formatNumber(value)}'];
    case PropCheckbox(:final value):
      return ['$key: $value'];
    case PropDate(:final value):
      return ['$key: $value'];
    case PropList(:final items):
      if (items.isEmpty) return ['$key: []'];
      if (blockList) {
        return ['$key:', ...items.map((it) => '  - ${_quoteIfNeeded(it)}')];
      }
      final inner = items.map(_quoteIfNeeded).join(', ');
      return ['$key: [$inner]'];
  }
}

/// Split `key: rest` at the first colon; key must be a plain unquoted scalar.
(String, String)? _splitKey(String raw) {
  final colon = raw.indexOf(':');
  if (colon < 0) return null;
  final key = raw.substring(0, colon);
  if (key.isEmpty ||
      key.contains(RegExp(r'\s')) ||
      key.contains('"') ||
      key.contains("'") ||
      key.contains('#')) {
    return null;
  }
  return (key, raw.substring(colon + 1));
}

/// Infer a typed value from a trimmed non-empty scalar. bool → number → date →
/// flow-list → text. Mirrors `infer_scalar`.
PropertyValue _inferScalar(String text) {
  switch (text) {
    case 'true' || 'True' || 'TRUE':
      return const PropCheckbox(true);
    case 'false' || 'False' || 'FALSE':
      return const PropCheckbox(false);
  }
  if (text.startsWith('[') && text.endsWith(']')) {
    final inner = text.substring(1, text.length - 1);
    final items = inner.trim().isEmpty ? <String>[] : _splitFlowItems(inner);
    return PropList(items);
  }
  if (!text.startsWith('"') && !text.startsWith("'")) {
    final n = double.tryParse(text);
    if (n != null && n.isFinite && _formatNumber(n) == text) {
      return PropNumber(n);
    }
  }
  if (_isIsoDate(text)) return PropDate(text);
  return PropText(_unquoteScalar(text));
}

/// `YYYY-MM-DD`, digits and month/day in range. Shape only. Mirrors `is_iso_date`.
bool _isIsoDate(String s) {
  if (s.length != 10 || s[4] != '-' || s[7] != '-') return false;
  for (var i = 0; i < 10; i++) {
    if (i == 4 || i == 7) continue;
    final c = s.codeUnitAt(i);
    if (c < 0x30 || c > 0x39) return false;
  }
  final month = s.substring(5, 7);
  final day = s.substring(8, 10);
  return month.compareTo('01') >= 0 &&
      month.compareTo('12') <= 0 &&
      day.compareTo('01') >= 0 &&
      day.compareTo('31') <= 0;
}

/// Split a flow-list body on top-level commas, respecting quotes. Mirrors
/// `split_flow_items`.
List<String> _splitFlowItems(String inner) {
  final items = <String>[];
  final cur = StringBuffer();
  var inDouble = false;
  var inSingle = false;
  var escaped = false;
  for (final c in inner.split('')) {
    if (inDouble) {
      cur.write(c);
      if (escaped) {
        escaped = false;
      } else if (c == '\\') {
        escaped = true;
      } else if (c == '"') {
        inDouble = false;
      }
    } else if (inSingle) {
      cur.write(c);
      if (c == "'") inSingle = false;
    } else {
      if (c == '"') {
        inDouble = true;
        cur.write(c);
      } else if (c == "'") {
        inSingle = true;
        cur.write(c);
      } else if (c == ',') {
        items.add(cur.toString());
        cur.clear();
      } else {
        cur.write(c);
      }
    }
  }
  items.add(cur.toString());
  return items.map((s) => _unquoteScalar(s.trim())).toList();
}

/// Strip one layer of matching quotes, unescaping `\"`/`\\`/`\n` in a double
/// quote and `''`→`'` in a single quote. Mirrors `unquote_scalar`.
String _unquoteScalar(String s) {
  if (s.length >= 2 && s.startsWith('"') && s.endsWith('"')) {
    final inner = s.substring(1, s.length - 1);
    final out = StringBuffer();
    var i = 0;
    while (i < inner.length) {
      final c = inner[i];
      if (c == '\\' && i + 1 < inner.length) {
        final n = inner[i + 1];
        if (n == '"') {
          out.write('"');
          i += 2;
          continue;
        } else if (n == '\\') {
          out.write('\\');
          i += 2;
          continue;
        } else if (n == 'n') {
          out.write('\n');
          i += 2;
          continue;
        }
        out.write('\\');
        i += 1;
      } else {
        out.write(c);
        i += 1;
      }
    }
    return out.toString();
  }
  if (s.length >= 2 && s.startsWith("'") && s.endsWith("'")) {
    return s.substring(1, s.length - 1).replaceAll("''", "'");
  }
  return s;
}

const _indicators = {
  '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '%', '@', '`', '"', "'",
  '-', '?', ':', ',',
};

/// A non-empty scalar re-parses back to exactly the same text (so rendering it
/// bare is safe). Mirrors the `infer_scalar` half of `quote_if_needed`.
bool _staysAsText(String s) {
  final infer = _inferScalar(s);
  return infer is PropText && infer.value == s;
}

/// Double-quote a string when leaving it bare would change its re-parse.
/// Mirrors `quote_if_needed`.
String _quoteIfNeeded(String s) {
  final needs = s.isEmpty ||
      s.startsWith(' ') ||
      s.startsWith('\t') ||
      s.endsWith(' ') ||
      s.endsWith('\t') ||
      (s.isNotEmpty && _indicators.contains(s[0])) ||
      s.contains(': ') ||
      s.contains(', ') ||
      s.contains('\n') ||
      s.contains(']') ||
      (s.isNotEmpty && !_staysAsText(s));
  if (needs) {
    final escaped =
        s.replaceAll('\\', '\\\\').replaceAll('"', '\\"').replaceAll('\n', '\\n');
    return '"$escaped"';
  }
  return s;
}

/// Format a double without a trailing `.0` for integers. Mirrors `format_number`.
String _formatNumber(double n) {
  if (n == n.truncateToDouble() && n.abs() < 1e15) {
    return n.toInt().toString();
  }
  return n.toString();
}
