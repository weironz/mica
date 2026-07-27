// What a rename should actually save.
//
// One function because the same three-part judgement was already written out
// twice — once for the sidebar row's inline rename, once for the page-title field
// — and the breadcrumb's rename would have been a third copy. Each part has a
// consequence, so they are worth pinning in one place with tests rather than
// re-deriving them per call site.

/// The name to save, or null when nothing should be saved.
///
/// Returns null in two cases, both of which the caller must treat as "leave it
/// alone" rather than as an error:
///
/// * **Blank.** The server rejects an empty name, and a page whose name is
///   whitespace renders as a dash — so a cleared field means "I changed my mind",
///   not "call the page nothing".
/// * **Unchanged.** Commit-on-blur means this fires every time the field is merely
///   visited; without the check, opening and closing a rename would send a write,
///   bump `updated_at`, and reshuffle "recently edited" for nothing.
///
/// Trimming is not cosmetic either: a trailing space would make the name differ
/// from the identical-looking one the user meant to keep.
String? renamedTo(String input, String currentName) {
  final trimmed = input.trim();
  if (trimmed.isEmpty) return null;
  if (trimmed == currentName) return null;
  return trimmed;
}
