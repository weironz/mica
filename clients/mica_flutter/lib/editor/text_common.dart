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
