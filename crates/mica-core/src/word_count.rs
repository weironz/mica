//! Single authoritative word & character counting for Mica (Issue #191).
//!
//! Shared across:
//! - Backend Postgres (`document_yrs_base.word_count` / `char_count`),
//! - Local SQLite (`doc_snapshot.word_count` / `char_count`),
//! - Editor live status bar (`clients/mica_rust/src/editor_bridge.rs`).
//!
//! Rules (§2.2 of `docs/features/usage-statistics.md`):
//! - Each CJK character (Han Unified Ideographs, CJK Extension A, Hiragana,
//!   Katakana, Hangul Syllables) counts as **1 word**.
//! - Each contiguous run of non-CJK alphanumeric characters (Latin letters,
//!   digits, accented/Cyrillic/Greek letters, etc.) counts as **1 word**.
//! - Punctuation and whitespace separate words and do not add to `words`.
//! - `chars` counts all non-whitespace Unicode scalar values (`!c.is_whitespace()`).

use crate::block::Block;
use crate::doc::MicaDoc;

/// Word and non-whitespace character totals for a text span, block slice, or document.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WordCount {
  pub words: u64,
  pub chars: u64,
}

impl WordCount {
  pub fn saturating_add(self, rhs: Self) -> Self {
    Self {
      words: self.words.saturating_add(rhs.words),
      chars: self.chars.saturating_add(rhs.chars),
    }
  }
}

/// Returns `true` if `c` is a CJK character that counts as an individual word:
/// CJK Unified Ideographs (`U+4E00..=U+9FFF`), Extension A (`U+3400..=U+4DBF`),
/// Hiragana & Katakana (`U+3040..=U+30FF`), Hangul Syllables (`U+AC00..=U+D7AF`),
/// or CJK Compatibility Ideographs (`U+F900..=U+FAFF`).
#[inline]
pub fn is_cjk_word_char(c: char) -> bool {
  matches!(
    c as u32,
    0x4E00..=0x9FFF
      | 0x3400..=0x4DBF
      | 0x3040..=0x30FF
      | 0xAC00..=0xD7AF
      | 0xF900..=0xFAFF
  )
}

/// Count words and non-whitespace characters in a single text string.
pub fn count_text(text: &str) -> WordCount {
  let mut words = 0u64;
  let mut chars = 0u64;
  let mut in_latin_word = false;

  for c in text.chars() {
    if !c.is_whitespace() {
      chars += 1;
    }
    if is_cjk_word_char(c) {
      words += 1;
      in_latin_word = false;
    } else if c.is_alphanumeric() {
      if !in_latin_word {
        words += 1;
        in_latin_word = true;
      }
    } else {
      in_latin_word = false;
    }
  }

  WordCount { words, chars }
}

/// Count words and non-whitespace characters across flattened document blocks.
///
/// Mirrors how `content_text` is projected from blocks: each block's `text`
/// field (excluding trailing newline artifacts) is counted independently so
/// adjacent blocks never merge two Latin words together. For table blocks whose
/// cell text lives in `data.rows`, each cell string is also counted.
pub fn count_blocks(blocks: &[Block]) -> WordCount {
  let mut total = WordCount::default();
  for block in blocks {
    let trimmed = block.text.trim_end_matches('\n');
    if !trimmed.is_empty() {
      total = total.saturating_add(count_text(trimmed));
    }
    if block.kind == "table" {
      if let Some(rows) = block.data.get("rows").and_then(|v| v.as_array()) {
        for row in rows {
          if let Some(cells) = row.as_array() {
            for cell in cells {
              if let Some(s) = cell.as_str() {
                if !s.is_empty() {
                  total = total.saturating_add(count_text(s));
                }
              }
            }
          }
        }
      }
    }
  }
  total
}

/// Count words and non-whitespace characters in a decoded [`MicaDoc`].
pub fn count_doc(doc: &MicaDoc) -> WordCount {
  let blocks = doc.to_blocks();
  count_blocks(&blocks)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn empty_and_whitespace_only() {
    assert_eq!(count_text(""), WordCount { words: 0, chars: 0 });
    assert_eq!(
      count_text("   \t\n\r\u{3000} "),
      WordCount { words: 0, chars: 0 }
    );
  }

  #[test]
  fn pure_chinese_text() {
    let wc = count_text("云母知识库，你好！");
    // 7 Han chars = 7 words; 7 Han + 2 punctuation = 9 non-whitespace chars.
    assert_eq!(wc, WordCount { words: 7, chars: 9 });
  }

  #[test]
  fn pure_english_and_numbers() {
    let wc = count_text("Hello, world! Mica v0.14.11 is fast.");
    // Words: Hello, world, Mica, v0, 14, 11, is, fast = 8
    assert_eq!(wc.words, 8);
    assert_eq!(
      wc.chars,
      "Hello,world!Micav0.14.11isfast.".chars().count() as u64
    );
  }

  #[test]
  fn mixed_cjk_latin_japanese_korean() {
    let wc = count_text("使用Rust编写Mica 2026 こんにちは 한글");
    // 使(1) 用(2) Rust(3) 编(4) 写(5) Mica(6) 2026(7) こ(8) ん(9) に(10) ち(11) は(12) 한(13) 글(14)
    assert_eq!(wc.words, 14);
    assert_eq!(
      wc.chars,
      "使用Rust编写Mica2026こんにちは한글".chars().count() as u64
    );
  }

  #[test]
  fn blocks_do_not_glue_adjacent_latin_words_and_include_table_cells() {
    let blocks = vec![
      Block {
        id: "b1".into(),
        kind: "paragraph".into(),
        text: "Hello\n".into(),
        data: json!({}),
        children: vec![],
      },
      Block {
        id: "b2".into(),
        kind: "paragraph".into(),
        text: "World".into(),
        data: json!({}),
        children: vec![],
      },
      Block {
        id: "t1".into(),
        kind: "table".into(),
        text: String::new(),
        data: json!({
          "rows": [
            ["姓名", "Role"],
            ["张三", "Admin"]
          ]
        }),
        children: vec![],
      },
    ];
    let wc = count_blocks(&blocks);
    // Hello(1) + World(1) + 姓(1)+名(1)+Role(1)+张(1)+三(1)+Admin(1) = 8 words
    assert_eq!(wc.words, 8);
    assert_eq!(wc.chars, 5 + 5 + 2 + 4 + 2 + 5);
  }
}
