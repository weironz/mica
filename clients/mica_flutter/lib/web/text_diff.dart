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

/// The one splice that turns [old] into [next], or null when they are equal.
///
/// A single common-prefix/common-suffix splice, deliberately — a keystroke IS
/// one splice, and a cleverer diff would buy nothing for the case that matters
/// while adding a way to be wrong.
TextSplice? textDiffUtf16(String old, String next) {
  if (old == next) return null;
  final a = old.codeUnits;
  final b = next.codeUnits;

  var p = 0;
  while (p < a.length && p < b.length && a[p] == b[p]) {
    p++;
  }
  // `a[p]` is where they first differ. If that lands on a low surrogate, the
  // pair started inside the common prefix — back off so the whole character is
  // on the changed side, instead of handing the CRDT half of one.
  if (p > 0 && p < a.length && _isLowSurrogate(a[p])) p--;

  var s = 0;
  while (s < a.length - p &&
      s < b.length - p &&
      a[a.length - 1 - s] == b[b.length - 1 - s]) {
    s++;
  }
  // The suffix begins at `a.length - s`; if that is a low surrogate its high
  // half is outside the suffix, so shrink the suffix to keep the pair whole.
  if (s > 0 && _isLowSurrogate(a[a.length - s])) s--;

  return TextSplice(p, a.length - p - s, next.substring(p, b.length - s));
}
