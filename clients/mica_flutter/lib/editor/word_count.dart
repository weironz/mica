/// Document word/character counts, computed over the plain text of the blocks.
///
/// Kept as a free function over `Iterable<String>` (not `EditorNode`) so it is
/// pure and unit-testable without the editor. The counting口径 is deliberately
/// simple and mixed-script friendly (see [countBlocks]).
class DocCounts {
  const DocCounts(this.words, this.chars);

  /// CJK ideographs/kana/hangul each count as one word; every maximal run of
  /// Latin/other letters+digits counts as one word. Punctuation, symbols and
  /// whitespace are word boundaries and are not words themselves.
  final int words;

  /// Every non-whitespace character (letters, digits, CJK, punctuation, symbols)
  /// counts once. Structural block newlines are whitespace and never counted.
  final int chars;

  static const DocCounts zero = DocCounts(0, 0);

  @override
  bool operator ==(Object other) =>
      other is DocCounts && other.words == words && other.chars == chars;

  @override
  int get hashCode => Object.hash(words, chars);

  @override
  String toString() => 'DocCounts(words: $words, chars: $chars)';
}

bool _isWhitespace(int c) =>
    c == 0x20 || // space
    c == 0x09 || // tab
    c == 0x0A || // LF
    c == 0x0D || // CR
    c == 0xA0 || // NBSP
    c == 0x3000; // ideographic space

/// CJK ideographs, kana, hangul syllables and compatibility ideographs — the
/// scripts where each character is its own "word". Ranges mirror the editor's
/// double-click word classifier (model.dart `_classOf`), plus the SMP ideograph
/// extensions so rare Han still counts as one-word-each.
bool _isCjk(int c) =>
    (c >= 0x3400 && c <= 0x9FFF) || // CJK Unified + Ext-A
    (c >= 0x3040 && c <= 0x30FF) || // hiragana + katakana
    (c >= 0xAC00 && c <= 0xD7A3) || // hangul syllables
    (c >= 0xF900 && c <= 0xFAFF) || // CJK compatibility ideographs
    (c >= 0x20000 && c <= 0x2FA1F); // SMP ideograph extensions

/// Non-ASCII punctuation/symbol blocks that must NOT join a word run — most
/// importantly CJK and fullwidth punctuation (、。，！？：；「」（）…), which the
/// "unknown non-ASCII is a letter" default in [_isWordChar] would otherwise
/// swallow into words. Kept to the ranges that actually occur in CJK/European
/// prose; exotic symbols/emoji are out of scope (they'd count as a word, which
/// is harmless for a rough badge).
bool _isNonAsciiSymbol(int c) =>
    (c >= 0x2000 && c <= 0x206F) || // general punctuation (– — " " … • ‹ ›)
    (c >= 0x3000 && c <= 0x303F) || // CJK symbols & punctuation (。、「」【】〜)
    (c >= 0xFE30 && c <= 0xFE4F) || // CJK compatibility forms
    (c >= 0xFF01 && c <= 0xFF0F) || // fullwidth ！＂＃＄％＆＇（）＊＋，－．／
    (c >= 0xFF1A && c <= 0xFF20) || // fullwidth ：；＜＝＞？＠
    (c >= 0xFF3B && c <= 0xFF40) || // fullwidth ［＼］＾＿｀
    (c >= 0xFF5B && c <= 0xFF65); // fullwidth ｛｜｝～ + halfwidth ｡｢｣､･

/// A character that participates in a Latin-style "word" run. ASCII letters and
/// digits qualify; a non-ASCII code point counts too UNLESS it is CJK (each of
/// those is its own word) or a known punctuation/symbol — so accented Latin,
/// Cyrillic, Greek, fullwidth letters/digits etc. read as letters, while CJK and
/// fullwidth punctuation break the run. ASCII punctuation/symbols also break it.
bool _isWordChar(int c) {
  if (c > 0x7F) return !_isCjk(c) && !_isNonAsciiSymbol(c);
  if (c >= 0x30 && c <= 0x39) return true; // 0-9
  if (c >= 0x41 && c <= 0x5A) return true; // A-Z
  if (c >= 0x61 && c <= 0x7A) return true; // a-z
  return false;
}

/// Count words and non-whitespace characters across [blocks] (each a block's
/// plain text). Every block boundary is treated as a break, so a Latin word
/// never spans two blocks. See [DocCounts] for the exact口径.
DocCounts countBlocks(Iterable<String> blocks) {
  var words = 0;
  var chars = 0;
  for (final block in blocks) {
    var inRun = false; // inside a Latin-style word run
    for (final c in block.runes) {
      if (_isWhitespace(c)) {
        if (inRun) {
          words++;
          inRun = false;
        }
        continue;
      }
      chars++;
      if (_isCjk(c)) {
        if (inRun) {
          words++;
          inRun = false;
        }
        words++; // each CJK glyph is one word
      } else if (_isWordChar(c)) {
        inRun = true;
      } else {
        // punctuation / symbol: counts as a char, breaks the run
        if (inRun) {
          words++;
          inRun = false;
        }
      }
    }
    if (inRun) words++; // block boundary closes an open run
  }
  return DocCounts(words, chars);
}
