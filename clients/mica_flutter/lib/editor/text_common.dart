// Common-prefix / common-suffix of two strings in UTF-16 code units — the one
// primitive the editor's mark-shifting callers (`controller.dart`,
// `cell_edit_controller.dart`, `prose_input.dart`) diff an old/new text pair
// with. Lived in `web/text_diff.dart` beside the yjs-side text splice until the
// web moved to the Rust document core (2026-11-01); the splice itself is now
// only `crates/mica-core/src/text_diff.rs`.
library;

/// Length of the longest common prefix of [a] and [b], in UTF-16 code units.
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

/// Compute the replaced range `(prefix, oldEnd)` in [oldText] for an edit to
/// [newText], biased toward the pre-edit selection `[prevStart, prevEnd]` when
/// repeated characters make the raw prefix/suffix diff ambiguous (for example
/// typing `h` before a link `https://...` at offset 0, or deleting a repeated
/// character next to a mark boundary; gh#192).
({int prefix, int oldEnd}) diffEditRange(
  String oldText,
  String newText, {
  int? prevStart,
  int? prevEnd,
  int? nextCaret,
}) {
  var prefix = commonPrefix(oldText, newText);
  var oldEnd = oldText.length - commonSuffix(oldText, newText, prefix);
  if (prevStart != null && prevEnd != null) {
    final s = prevStart.clamp(0, oldText.length);
    final e = prevEnd.clamp(s, oldText.length);
    if (s != e) {
      if (prefix > s) prefix = s;
      if (oldEnd < e) oldEnd = e;
    } else {
      final delta = newText.length - oldText.length;
      if (delta > 0 &&
          oldEnd == prefix &&
          prefix > s &&
          (nextCaret == null || nextCaret == s + delta) &&
          oldText.substring(s) == newText.substring(s + delta)) {
        prefix = s;
        oldEnd = s;
      } else if (delta < 0 &&
          nextCaret != null &&
          nextCaret == s + delta &&
          nextCaret >= 0 &&
          prefix > nextCaret &&
          oldText.substring(0, nextCaret) == newText.substring(0, nextCaret) &&
          oldText.substring(s) == newText.substring(nextCaret)) {
        prefix = nextCaret;
        oldEnd = s;
      }
    }
  }
  return (prefix: prefix, oldEnd: oldEnd);
}
