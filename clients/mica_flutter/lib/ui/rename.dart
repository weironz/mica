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

/// What the page-title field should do with the title the DOCUMENT now states.
///
/// See [titleFieldSync].
enum TitleFieldSync {
  /// Leave the field alone — it holds keystrokes the document has not seen yet.
  keepField,

  /// The document has caught up with the field. Nothing to write.
  settled,

  /// Overwrite the field with the document's title.
  takeDocument,
}

/// Whether an incoming document title may be written into the page-title field.
///
/// The field is edited live but saved on a debounce, and the save round-trips
/// through the document (since P2 the title lives in the document and
/// `views.name` is its projection) before echoing back as a CRDT update. So the
/// field and the document are routinely out of step, and the naive rule — "the
/// document is the authority, so assign it" — EATS INPUT: type a long title,
/// the debounce commits the first half, and while you are still typing the
/// second half that half-title arrives back and replaces the whole field.
/// Reported as "editing the title auto-refreshes the page and part of what I
/// typed disappears".
///
/// Which side wins therefore turns on whether the field has an edit in flight,
/// not on who spoke last:
///
/// * **Already equal (including normalized saves)** → [TitleFieldSync.settled]. Nothing to write, and
///   writing anyway is not free: assigning `.text` resets the selection, which
///   the web engine renders as select-all (one backspace would then wipe the
///   name) and which drops an in-progress IME composition.
/// * **Another page opened** ([pageChanged]) → [TitleFieldSync.takeDocument].
///   The caller flushes the departing page's debounce, and the field must show
///   the page now on screen.
/// * **Same page, edit pending** → [TitleFieldSync.keepField]. The field is
///   ahead of the document, which is precisely the state the echo of our own
///   save arrives in.
/// * **Otherwise** → [TitleFieldSync.takeDocument]: a rename made elsewhere
///   (another device, the sidebar row, the breadcrumb) that the field has
///   nothing newer to lose to.
///
/// Both non-[TitleFieldSync.keepField] answers mean the field no longer holds
/// anything uncommitted, so the caller clears its pending latch on either.
TitleFieldSync titleFieldSync({
  required String field,
  required String documentTitle,
  String? displayTitle,
  required bool pageChanged,
  required bool editPending,
}) {
  if (field == (displayTitle ?? documentTitle)) return TitleFieldSync.settled;
  if (pageChanged) return TitleFieldSync.takeDocument;
  // Saves normalize outer whitespace. Acknowledging that value must release
  // the pending edit without resetting the caret or an IME composition.
  if (field.trim() == documentTitle) return TitleFieldSync.settled;
  if (editPending) return TitleFieldSync.keepField;
  return TitleFieldSync.takeDocument;
}
