import 'package:flutter/widgets.dart';

import '../markdown/markdown_codec.dart';
import '../ui/theme_tokens.dart';

/// Code-block highlighting for the editor: colours and memos only.
///
/// The tokenizer, the grammars and language detection live in the engine
/// (`crates/md-core/src/highlight`), the one copy the HTML export and the share
/// page use too — they used to be here, so the editor coloured code and every
/// exported or shared page showed it flat (gh#178). With no engine (a widget
/// test that installs none, a failed wasm load) code simply renders plain.

/// The engine's answer to one `highlight` request, or null without an engine.
Object? _ask(Map<String, Object?> request) {
  final codec = markdownCodecOrNull;
  if (codec == null) return null;
  return withEngineOrNull<Object?>(() => codec.highlight(request));
}

List<String>? _languages;

/// Languages offered in the code-block picker; `auto` detects from content.
/// Without an engine only `auto` and `plaintext` — nothing else could colour.
List<String> get codeLanguages {
  final cached = _languages;
  if (cached != null) return cached;
  final answer = _ask({'op': 'languages'});
  if (answer is! List) return const ['auto', 'plaintext'];
  return _languages = List.unmodifiable(answer.cast<String>());
}

final Map<String, String> _canonicalMemo = {};

/// The canonical name for a fence label / picker choice: `py` → `python`.
/// Unknown names pass through trimmed and lowercased (and won't highlight).
String canonicalCodeLanguage(String name) {
  final hit = _canonicalMemo[name];
  if (hit != null) return hit;
  final answer = _ask({'op': 'canonical', 'name': name});
  if (answer is! String) return name.trim().toLowerCase();
  if (_canonicalMemo.length >= 256) _canonicalMemo.clear();
  return _canonicalMemo[name] = answer;
}

/// The language whose structural syntax the code plainly uses (a shebang,
/// `def f():`), or null. Strict on purpose: it overrules a pasted fence's own
/// label (`retagMislabeledFences`), so null is always the safe answer.
String? strongLanguageSignature(String code) {
  final answer = _ask({'op': 'signature', 'code': code});
  return answer is String ? answer : null;
}

/// Heuristic language detection; `plaintext` when nothing fits.
String detectLanguage(String code) {
  if (code.isEmpty) return 'plaintext';
  final answer = _ask({'op': 'detect', 'code': code});
  return answer is String ? answer : 'plaintext';
}

/// Resolve the effective language: explicit choice, or auto-detected when the
/// choice is null/empty/`auto`. The auto branch is memoized (see
/// [_ResolveLangKey]).
String resolveCodeLanguage(String code, String? selected) {
  final choice = canonicalCodeLanguage(selected ?? '');
  if (choice.isNotEmpty && choice != 'auto') return choice;
  final key = _ResolveLangKey(code);
  final hit = _resolveLangMemo.remove(key);
  if (hit != null) {
    _resolveLangMemo[key] = hit; // reinsert => most-recently-used
    return hit;
  }
  final lang = detectLanguage(code);
  _resolveLangMemo[key] = lang;
  if (_resolveLangMemo.length > _resolveLangMemoMax) {
    _resolveLangMemo.remove(_resolveLangMemo.keys.first); // evict LRU
  }
  return lang;
}

/// Memo for [resolveCodeLanguage]'s auto branch. Detection scans the whole
/// block with dozens of regexes, and the editor re-resolves on every caret move
/// inside a code block (a block holding the caret is excluded from render.dart's
/// whole-layout reuse). Identity IS freshness: the controller reassigns a
/// block's text instance on every edit, so an unchanged block always keys on
/// the same String object. Small bounded LRU, same discipline as
/// [_codeSpanMemo].
class _ResolveLangKey {
  const _ResolveLangKey(this.code);
  final String code;
  @override
  bool operator ==(Object other) =>
      other is _ResolveLangKey && identical(other.code, code);
  @override
  int get hashCode => identityHashCode(code);
}

final Map<_ResolveLangKey, String> _resolveLangMemo = {};
const int _resolveLangMemoMax = 128;

/// Memo for [buildCodeSpan]. The editor rebuilds every code block's span on
/// every keystroke — render.dart's Phase-1 painter cache always builds the span
/// in order to compare it — so unchanged blocks would re-tokenize (now: cross
/// the engine boundary) for nothing. A code span depends ONLY on (code,
/// language, base, palette): a code block carries no inline marks, so the key
/// is complete and can never go stale. Small bounded LRU; an unchanged block
/// hits in ~O(1) (String.hashCode is cached on the instance, and TextSpan.==
/// short-circuits on identity).
class _CodeSpanKey {
  const _CodeSpanKey(this.code, this.language, this.base, this.colors);
  final String code;
  final String language;
  final TextStyle base;

  /// Part of the key, not a detail: the spans carry resolved colours, so a hit
  /// from the other palette would repaint the block in the wrong theme and keep
  /// doing it until the source changed.
  final CodeTokens colors;
  @override
  bool operator ==(Object other) =>
      other is _CodeSpanKey &&
      other.code == code &&
      other.language == language &&
      other.base == base &&
      identical(other.colors, colors);
  @override
  int get hashCode =>
      Object.hash(code, language, base, identityHashCode(colors));
}

final Map<_CodeSpanKey, TextSpan> _codeSpanMemo = {};
const int _codeSpanMemoMax = 128;

/// Highlighted span for one code block, memoized (see [_CodeSpanKey]). The
/// returned span is immutable and may be shared across painters — safe, and it
/// makes the Phase-1 span-equality check identity-fast for unchanged blocks.
TextSpan buildCodeSpan(
  String code,
  String language,
  TextStyle base,
  CodeTokens colors,
) {
  final key = _CodeSpanKey(code, language, base, colors);
  final hit = _codeSpanMemo.remove(key);
  if (hit != null) {
    _codeSpanMemo[key] = hit; // reinsert => most-recently-used
    return hit;
  }
  final span = _buildCodeSpanUncached(code, language, base, colors);
  _codeSpanMemo[key] = span;
  if (_codeSpanMemo.length > _codeSpanMemoMax) {
    _codeSpanMemo.remove(_codeSpanMemo.keys.first); // evict least-recently-used
  }
  return span;
}

/// Engine role index (wire order) → palette colour.
Color _roleColor(int role, CodeTokens c) => switch (role) {
  0 => c.keyword,
  1 => c.string,
  2 => c.comment,
  3 => c.number,
  4 => c.function,
  _ => c.key,
};

TextSpan _buildCodeSpanUncached(
  String code,
  String language,
  TextStyle base,
  CodeTokens colors,
) {
  final name = canonicalCodeLanguage(language);
  if (name == 'plaintext' || !codeLanguages.contains(name)) {
    return TextSpan(text: code, style: base);
  }
  final answer = _ask({'op': 'tokens', 'code': code, 'language': name});
  if (answer is! List) return TextSpan(text: code, style: base);
  // Flat [start, end, role, …] in UTF-16 units — Dart's own string indices.
  final flat = answer.cast<int>();
  final spans = <TextSpan>[];
  var pos = 0;
  for (var t = 0; t + 2 < flat.length; t += 3) {
    final start = flat[t], end = flat[t + 1];
    if (start > pos) {
      spans.add(TextSpan(text: code.substring(pos, start), style: base));
    }
    spans.add(
      TextSpan(
        text: code.substring(start, end),
        style: base.copyWith(color: _roleColor(flat[t + 2], colors)),
      ),
    );
    pos = end;
  }
  if (pos < code.length) {
    spans.add(TextSpan(text: code.substring(pos), style: base));
  }
  return TextSpan(style: base, children: spans);
}
