// Recognizing a self-contained `<details>` element inside a raw-HTML block.
//
// `<details><summary>` is legal CommonMark/GFM raw HTML — GitHub's own docs
// recommend it for collapsible sections — so it arrives as an ordinary
// `code_block` with `data.raw == true` and the tags in `node.text`. Nothing
// here changes that: parsing and serialization are untouched, round-trip is
// byte-identical. This is only the recognizer that lets the RENDERER show a
// fold instead of four lines of source.
//
// TWO forms are recognized, by two different functions.
//
// **Tight** — the whole element in one block, read by [parseDetailsBlock]:
//
//     <details>
//     <summary>label</summary>
//     body
//     </details>
//
// **Blank-line** — the form GitHub's docs actually recommend, which puts a
// blank line before the body so the body is parsed as Markdown. A blank line
// ends a type-6 HTML block, so this arrives as N+2 blocks: the opening tags
// ([parseDetailsOpenTag]), the real Markdown body, and a bare `</details>`
// ([isDetailsCloseTag]). Verified against the authoritative Rust parser
// (`crates/markdown`) — see `test/details_fold_test.dart` for the block shapes
// that test pins.
//
// Folding the blank-line form hides a RANGE of nodes, which the renderer does
// with zero-height "absorbed" layouts rather than by skipping nodes —
// `_layouts` is indexed by node position throughout render.dart. See
// docs/render-architecture.md.
library;

/// A `<details>` element that this file was able to read whole.
class DetailsShape {
  const DetailsShape({
    required this.summary,
    required this.body,
    required this.openByDefault,
  });

  /// The `<summary>` text, or '' when the element has none (HTML then shows a
  /// browser default like "Details"; the renderer supplies its own label).
  final String summary;

  /// Everything between the summary and `</details>`, verbatim. May be ''.
  ///
  /// In the tight form this is NOT Markdown — no blank line means the parser
  /// swallowed it as raw HTML — so the renderer shows it as-is rather than
  /// pretending to have rendered it.
  final String body;

  /// `<details open>` starts expanded, plain `<details>` starts folded. This
  /// is only the DEFAULT: `data.collapsed`, when present, is the user's own
  /// choice and wins (same tri-state as a long code block).
  final bool openByDefault;
}

final _open = RegExp(
  r'^<details(\s+open(\s*=\s*("open"|' "'open'" r'|""|' "''" r'))?)?\s*>$',
  caseSensitive: false,
);
final _summary = RegExp(
  r'^<summary\s*>(.*)</summary\s*>$',
  caseSensitive: false,
);

/// Read [text] as one whole `<details>` element, or null to leave it alone.
///
/// Deliberately strict: anything it does not recognize keeps rendering as the
/// raw source it is today, which is never wrong — only plain. In particular a
/// NESTED `<details>` declines, because the fold would then have to know where
/// the inner one ends.
DetailsShape? parseDetailsBlock(String text) {
  final lines = text.split('\n');
  // Trailing blank lines are noise; leading ones mean this isn't the element.
  while (lines.isNotEmpty && lines.last.trim().isEmpty) {
    lines.removeLast();
  }
  if (lines.length < 2) return null;

  final head = _open.firstMatch(lines.first.trim());
  if (head == null) return null;
  if (lines.last.trim().toLowerCase() != '</details>') return null;

  final inner = lines.sublist(1, lines.length - 1);
  // A second `<details>` (or a stray `</details>`) inside means this block
  // holds more than one element — out of scope, show the source.
  for (final l in inner) {
    final t = l.trim().toLowerCase();
    if (t.startsWith('<details') || t == '</details>') return null;
  }

  var summary = '';
  var bodyFrom = 0;
  if (inner.isNotEmpty) {
    final m = _summary.firstMatch(inner.first.trim());
    if (m != null) {
      summary = m.group(1)!.trim();
      bodyFrom = 1;
    }
  }
  // A stray `<summary>` further down is the same "more than one element"
  // problem as nested details.
  for (final l in inner.sublist(bodyFrom)) {
    if (l.trim().toLowerCase().startsWith('<summary')) return null;
  }

  return DetailsShape(
    summary: summary,
    body: inner.sublist(bodyFrom).join('\n').trim(),
    openByDefault: head.group(1) != null,
  );
}

/// The opening half of a blank-line-form `<details>` — the tags only. The body
/// lives in the blocks that follow, and the element ends at a separate
/// `</details>` block ([isDetailsCloseTag]).
class DetailsOpenTag {
  const DetailsOpenTag({required this.summary, required this.openByDefault});

  /// The `<summary>` text, or '' when the element has none.
  final String summary;

  /// See [DetailsShape.openByDefault] — same tri-state with `data.collapsed`.
  final bool openByDefault;
}

/// Read [text] as JUST the opening tags of a blank-line-form `<details>`, or
/// null to leave the block alone.
///
/// Strict on purpose, and strictly disjoint from [parseDetailsBlock]: a block
/// containing `</details>` is never an opener (that is the tight form, or
/// something this file does not understand). Accepted shapes are exactly the
/// two the parser produces for GitHub's recommended source:
///
///     <details>
///     <details open>
///     <details>\n<summary>label</summary>
///
/// Anything else — extra HTML lines, a multi-line `<summary>`, a nested
/// `<details` — declines, and the block keeps rendering as the source it is.
DetailsOpenTag? parseDetailsOpenTag(String text) {
  final lines = text.split('\n');
  while (lines.isNotEmpty && lines.last.trim().isEmpty) {
    lines.removeLast();
  }
  if (lines.isEmpty || lines.length > 2) return null;

  final head = _open.firstMatch(lines.first.trim());
  if (head == null) return null;

  var summary = '';
  if (lines.length == 2) {
    final m = _summary.firstMatch(lines[1].trim());
    if (m == null) return null;
    summary = m.group(1)!.trim();
  }
  return DetailsOpenTag(summary: summary, openByDefault: head.group(1) != null);
}

final _close = RegExp(r'^</details\s*>$', caseSensitive: false);

/// True when [text] is nothing but a `</details>` closing tag — the block the
/// parser emits for the last line of the blank-line form.
bool isDetailsCloseTag(String text) {
  final lines = text.split('\n');
  while (lines.isNotEmpty && lines.last.trim().isEmpty) {
    lines.removeLast();
  }
  return lines.length == 1 && _close.hasMatch(lines.first.trim());
}

/// Whether a `<details>` block starts expanded, in EITHER form, or null when
/// [text] is not a `<details>` opener at all.
///
/// One function because two callers need the same answer and reading the wrong
/// default makes the first click on a fold a visible no-op (it writes the state
/// the block was already in).
bool? detailsOpenByDefault(String text) =>
    parseDetailsBlock(text)?.openByDefault ??
    parseDetailsOpenTag(text)?.openByDefault;
