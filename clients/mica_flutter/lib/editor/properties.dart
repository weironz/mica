/// Structured page properties over the document root's raw front-matter string.
///
/// The rules — which lines are properties, how a scalar infers its type, how a
/// value is written back — live ONLY in the Rust engine
/// (`crates/md-core/src/properties.rs`), reached through [MarkdownCodec]
/// (FFI on desktop, wasm on the web). This file holds the typed values the UI
/// works with and thin wrappers over the codec. Until 2026-10-31 it was a
/// hand-written Dart mirror of the parser and writer that had to be kept in
/// step by hand; it is not anymore — do not add front-matter syntax here.
///
/// Front matter stays the SOLE authority: it lives verbatim as the raw inner
/// string on the document root block's `data['front_matter']`. These functions
/// are a lazy view + surgical edit over that string — no second representation.
/// See docs/features/page-properties.md for the round-trip-invariant relaxation
/// (byte-exact → normalized-subset) this enables.
library;

import '../markdown/markdown_codec.dart';

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
/// fences). Empty when the engine is unavailable: the panel then shows no
/// properties rather than failing to draw.
List<Property> parseProperties(String frontMatter) {
  final answer = withEngineOrNull(
    () => markdownCodec.properties({'op': 'parse', 'front_matter': frontMatter}),
  );
  if (answer is! List) return const [];
  return [
    for (final item in answer.cast<Map<String, dynamic>>())
      Property(
        item['key'] as String,
        PropertyValue.fromJson(item['value'] as Map<String, dynamic>),
      ),
  ];
}

/// Infer a typed value from a user's raw single-line input (what the property
/// editor commits): empty → empty text, otherwise the engine's scalar
/// inference. Throws [MarkdownCodecException] when the engine is unavailable.
PropertyValue inferValue(String raw) => PropertyValue.fromJson(
  markdownCodec.properties({'op': 'infer', 'raw': raw})! as Map<String, dynamic>,
);

/// Insert or replace `key`'s value, editing only that key's line(s) and leaving
/// the rest byte-exact; a new key is appended. Throws
/// [MarkdownCodecException] when the engine is unavailable — an edit is never
/// silently dropped by answering the old text.
String upsertProperty(String frontMatter, String key, PropertyValue value) =>
    markdownCodec.properties({
          'op': 'upsert',
          'front_matter': frontMatter,
          'key': key,
          'value': value.toJson(),
        })!
        as String;

/// Remove `key` and its block-list lines, leaving the rest byte-exact. Throws
/// [MarkdownCodecException] when the engine is unavailable.
String removeProperty(String frontMatter, String key) =>
    markdownCodec.properties({
          'op': 'remove',
          'front_matter': frontMatter,
          'key': key,
        })!
        as String;
