// Recognizing a self-contained `<details>` element inside a raw-HTML block.
//
// `<details><summary>` is legal CommonMark/GFM raw HTML — GitHub's own docs
// recommend it for collapsible sections — so it arrives as an ordinary
// `code_block` with `data.raw == true` and the tags in `node.text`. Nothing
// here changes that: parsing and serialization are untouched, round-trip is
// byte-identical. This is only the recognizer that lets the RENDERER show a
// fold instead of four lines of source.
//
// **Only the tight form is recognized** — the whole element in one block:
//
//     <details>
//     <summary>label</summary>
//     body
//     </details>
//
// The form GitHub's docs actually recommend puts a BLANK LINE before the body
// so the body is parsed as Markdown. A blank line ends a type-6 HTML block, so
// that form arrives as THREE blocks (`<details>…<summary>`, the real Markdown
// body, `</details>`) and folding it means hiding a RANGE of nodes — a
// different mechanism, since `_layouts` is indexed by node position throughout
// the renderer. See docs/render-architecture.md and the toggle entry in
// docs/roadmap.md.
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
