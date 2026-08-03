//! The minimal edit that turns one block's text into another.
//!
//! The editor's hot path sends a block's WHOLE new text (`update_block`), and
//! both CRDT mirrors used to land it as "delete the entire range, insert the
//! new string". That is correct for one writer and wrong for two: a whole-range
//! replacement is not an edit yrs/yjs can interleave, so two people typing into
//! one paragraph converge on a DUPLICATE rather than on both edits —
//! `"Hello"` + A typing at the front + B typing at the end became
//! `"AHelloHelloB"` (measured; `tests/sync.rs`).
//!
//! Reducing the write to the characters that actually changed is what makes the
//! merge work, because then the two writers are touching disjoint ranges. The
//! fine-grained API (`text_insert`/`text_delete`) always did the right thing —
//! the editor just never used it.
//!
//! **Offsets are UTF-16 code units**, because the doc runs
//! `OffsetKind::Utf16` (so they equal Dart string indices). The boundaries are
//! nudged off surrogate pairs: cutting between a high and a low surrogate would
//! hand yrs half a character and corrupt any text outside the BMP — emoji, and
//! plenty of CJK extension B.
//!
//! Mirrored in Dart at `clients/mica_flutter/lib/web/mica_ydoc.dart`
//! (`textDiffUtf16`) for the web replica, which is yjs rather than yrs. Same
//! table of cases on both sides — this is the shape red line #1 is about.

/// A single splice: remove [`TextDiff::del`] units at [`TextDiff::at`], then
/// insert [`TextDiff::ins`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextDiff {
    /// Start offset, in UTF-16 code units.
    pub at: u32,
    /// How many UTF-16 code units to remove.
    pub del: u32,
    /// What to insert there.
    pub ins: String,
}

fn is_low_surrogate(u: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&u)
}

/// The one splice that turns `old` into `new`, or `None` when they are equal.
///
/// Deliberately a single common-prefix/common-suffix splice rather than a real
/// diff: a keystroke IS one splice, and anything fancier would buy nothing for
/// the case that matters while adding a way to be wrong.
pub fn utf16_text_diff(old: &str, new: &str) -> Option<TextDiff> {
    if old == new {
        return None;
    }
    let a: Vec<u16> = old.encode_utf16().collect();
    let b: Vec<u16> = new.encode_utf16().collect();

    let mut p = 0usize;
    while p < a.len() && p < b.len() && a[p] == b[p] {
        p += 1;
    }
    // `a[p]` is where they first differ. If that lands on a low surrogate, the
    // pair started inside the common prefix — back off so the whole character
    // is on the changed side.
    if p > 0 && p < a.len() && is_low_surrogate(a[p]) {
        p -= 1;
    }

    let mut s = 0usize;
    while s < a.len() - p && s < b.len() - p && a[a.len() - 1 - s] == b[b.len() - 1 - s] {
        s += 1;
    }
    // The suffix begins at `a.len() - s`. If that is a low surrogate its high
    // half is outside the suffix, so shrink the suffix to keep the pair whole.
    if s > 0 && is_low_surrogate(a[a.len() - s]) {
        s -= 1;
    }

    let ins = String::from_utf16(&b[p..b.len() - s]).ok()?;
    Some(TextDiff {
        at: p as u32,
        del: (a.len() - p - s) as u32,
        ins,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diff(old: &str, new: &str) -> Option<(u32, u32, String)> {
        utf16_text_diff(old, new).map(|d| (d.at, d.del, d.ins))
    }

    /// Applying the splice must reproduce `new` exactly — the property that
    /// makes every other assertion here safe to read.
    fn applied(old: &str, new: &str) -> String {
        let Some(d) = utf16_text_diff(old, new) else {
            return old.to_string();
        };
        let a: Vec<u16> = old.encode_utf16().collect();
        let mut out: Vec<u16> = a[..d.at as usize].to_vec();
        out.extend(d.ins.encode_utf16());
        out.extend_from_slice(&a[(d.at + d.del) as usize..]);
        String::from_utf16(&out).unwrap()
    }

    #[test]
    fn no_change_is_no_write() {
        assert_eq!(diff("Hello", "Hello"), None);
        assert_eq!(diff("", ""), None);
    }

    /// The two edits from the concurrency case, each seen on its own replica.
    #[test]
    fn a_keystroke_is_one_character() {
        assert_eq!(diff("Hello", "AHello"), Some((0, 0, "A".into())));
        assert_eq!(diff("Hello", "HelloB"), Some((5, 0, "B".into())));
        assert_eq!(diff("Hello", "HeXllo"), Some((2, 0, "X".into())));
    }

    #[test]
    fn deletion_and_replacement() {
        assert_eq!(diff("Hello", "Helo"), Some((3, 1, String::new())));
        assert_eq!(diff("Hello", ""), Some((0, 5, String::new())));
        assert_eq!(diff("", "Hi"), Some((0, 0, "Hi".into())));
        assert_eq!(diff("abc", "aXc"), Some((1, 1, "X".into())));
    }

    /// CJK inside the BMP is 1 unit per character, so offsets are character
    /// counts here — the ordinary case for this product.
    #[test]
    fn cjk_offsets_are_units_not_bytes() {
        assert_eq!(diff("你好", "你好吗"), Some((2, 0, "吗".into())));
        assert_eq!(diff("你好", "你们好"), Some((1, 0, "们".into())));
        assert_eq!(applied("笔记软件", "笔记软件很好"), "笔记软件很好");
    }

    /// Outside the BMP one character is TWO units. A boundary that lands
    /// between them would hand the CRDT half a character; these pin that it
    /// never does.
    #[test]
    fn surrogate_pairs_are_never_split() {
        // Same leading surrogate, different trailing one: the naive prefix
        // stops between the halves.
        let d = utf16_text_diff("😀", "😁").unwrap();
        assert_eq!(d.at, 0, "the whole character is replaced, not half of it");
        assert_eq!(d.del, 2);
        assert_eq!(d.ins, "😁");
        assert_eq!(applied("😀", "😁"), "😁");

        assert_eq!(applied("a😀b", "a😀Xb"), "a😀Xb");
        assert_eq!(applied("😀😀", "😀"), "😀");
        assert_eq!(applied("hi 😀", "hi 😀!"), "hi 😀!");
        // A trailing pair shared as the SUFFIX.
        assert_eq!(applied("x😀", "y😀"), "y😀");
    }

    /// Whatever the inputs, the splice has to reproduce the target. Cheap
    /// exhaustive sweep over a small alphabet that includes astral characters.
    #[test]
    fn the_splice_always_reproduces_the_target() {
        let alphabet = ["", "a", "ab", "你", "你好", "😀", "a😀", "😀a", "😀😀"];
        for old in alphabet {
            for new in alphabet {
                assert_eq!(applied(old, new), new, "{old:?} -> {new:?}");
            }
        }
    }
}
