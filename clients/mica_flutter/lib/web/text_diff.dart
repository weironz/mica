// The minimal edit that turns one block's text into another — the Dart mirror
// of Rust `crates/mica-core/src/text_diff.rs`.
//
// Two copies exist because the two CRDT engines do: desktop/server run yrs
// (Rust), web runs yjs (JS). They must land the SAME splice, or a desktop and a
// web client editing one paragraph would disagree about what their updates
// mean. Kept honest by the same table of cases on both sides —
// `text_diff.rs`'s unit tests and `test/text_diff_test.dart`.
//
// Offsets are UTF-16 code units. That is free here (a Dart String IS UTF-16),
// and is why the Rust side goes out of its way to match: the doc runs
// `OffsetKind::Utf16`.
library;

/// A single splice: remove [del] units at [at], then insert [ins].
class TextSplice {
  const TextSplice(this.at, this.del, this.ins);
  final int at;
  final int del;
  final String ins;

  @override
  bool operator ==(Object other) =>
      other is TextSplice &&
      other.at == at &&
      other.del == del &&
      other.ins == ins;

  @override
  int get hashCode => Object.hash(at, del, ins);

  @override
  String toString() => 'TextSplice($at, $del, ${ins.length} units)';
}

bool _isLowSurrogate(int u) => u >= 0xDC00 && u <= 0xDFFF;

/// Length of the longest common prefix of [a] and [b], in UTF-16 code units.
///
/// Shared with the editor's mark-shifting callers (`controller.dart`,
/// `cell_edit_controller.dart`), which diff an old/new text pair exactly the
/// way `textDiffUtf16` does — one primitive, not three drift-prone copies.
int commonPrefix(String a, String b) {
  final n = a.length < b.length ? a.length : b.length;
  var i = 0;
  while (i < n && a.codeUnitAt(i) == b.codeUnitAt(i)) {
    i++;
  }
  return i;
}

/// Length of the longest common suffix of [a] and [b] that starts at or after
/// [prefix] (so the two never overlap).
int commonSuffix(String a, String b, int prefix) {
  final max = (a.length < b.length ? a.length : b.length) - prefix;
  var i = 0;
  while (i < max &&
      a.codeUnitAt(a.length - 1 - i) == b.codeUnitAt(b.length - 1 - i)) {
    i++;
  }
  return i;
}

/// The one splice that turns [old] into [next], or null when they are equal.
///
/// A single common-prefix/common-suffix splice, deliberately — a keystroke IS
/// one splice, and a cleverer diff would buy nothing for the case that matters
/// while adding a way to be wrong.
TextSplice? textDiffUtf16(String old, String next) {
  if (old == next) return null;

  var p = commonPrefix(old, next);
  // `old[p]` is where they first differ. If that lands on a low surrogate, the
  // pair started inside the common prefix — back off so the whole character is
  // on the changed side, instead of handing the CRDT half of one.
  if (p > 0 && p < old.length && _isLowSurrogate(old.codeUnitAt(p))) p--;

  var s = commonSuffix(old, next, p);
  // The suffix begins at `old.length - s`; if that is a low surrogate its high
  // half is outside the suffix, so shrink the suffix to keep the pair whole.
  if (s > 0 && _isLowSurrogate(old.codeUnitAt(old.length - s))) s--;

  return TextSplice(p, old.length - p - s, next.substring(p, next.length - s));
}
