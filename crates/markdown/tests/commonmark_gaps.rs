//! Regression tests for the CommonMark P1 gap-closing work (scoreboard
//! 589 → 620). Each case pins one fixed spec example by its markdown→HTML
//! pair so a future change can't silently re-break it. The grouped examples
//! mirror the sections in docs/commonmark-scoreboard.md.

use mica_markdown::{export_html, import_markdown};

/// Assert the engine renders `md` to exactly `expected` (trailing newline of
/// the export forgiven, matching the official runner's normalization).
fn html_eq(md: &str, expected: &str) {
  let got = export_html(&import_markdown(md, "root")).unwrap_or_default();
  assert_eq!(got.trim_end(), expected, "markdown: {md:?}");
}

#[test]
fn emphasis_nesting_order() {
  // `***foo***` is em-outside-strong, not the reverse (spec ex. 416/467/468).
  html_eq("foo***bar***baz\n", "<p>foo<em><strong>bar</strong></em>baz</p>");
  html_eq("***foo***\n", "<p><em><strong>foo</strong></em></p>");
  html_eq(
    "_____foo_____\n",
    "<p><em><strong><strong>foo</strong></strong></em></p>",
  );
  // A link inside emphasis keeps the anchor inside the <em> (ex. 419/433).
  html_eq(
    "*foo [*bar*](/url)*\n",
    "<p><em>foo <a href=\"/url\"><em>bar</em></a></em></p>",
  );
}

#[test]
fn links_no_nested_link_in_text() {
  // A link's text may not contain a link — the outer brackets stay literal
  // and the inner link wins (spec ex. 518/519/532/533).
  html_eq(
    "[foo [bar](/uri)](/uri)\n",
    "<p>[foo <a href=\"/uri\">bar</a>](/uri)</p>",
  );
  html_eq(
    "[foo *[bar [baz](/uri)](/uri)*](/uri)\n",
    "<p>[foo <em>[bar <a href=\"/uri\">baz</a>](/uri)</em>](/uri)</p>",
  );
}

#[test]
fn links_inner_spans_bind_tighter_than_brackets() {
  // Code spans, autolinks and raw HTML swallow a `]` so it can't close a link
  // (spec ex. 524/525/526).
  html_eq("[foo`](/uri)`\n", "<p>[foo<code>](/uri)</code></p>");
  html_eq(
    "[foo <bar attr=\"](baz)\">\n",
    "<p>[foo <bar attr=\"](baz)\"></p>",
  );
  html_eq(
    "[foo<https://example.com/?search=](uri)>\n",
    "<p>[foo<a href=\"https://example.com/?search=%5D(uri)\">https://example.com/?search=](uri)</a></p>",
  );
}

#[test]
fn links_empty_text_and_linked_image() {
  // Empty link text is a valid (empty) anchor (spec ex. 484/487).
  html_eq("[](./target.md)\n", "<p><a href=\"./target.md\"></a></p>");
  html_eq("[]()\n", "<p><a href=\"\"></a></p>");
  // An image inside link text stays an anchored image, not a bare image
  // (spec ex. 517/531).
  html_eq(
    "[![moon](moon.jpg)](/uri)\n",
    "<p><a href=\"/uri\"><img src=\"moon.jpg\" alt=\"moon\" /></a></p>",
  );
}

#[test]
fn links_destination_and_title_escaping() {
  // An escaped `>` inside an angle destination is unterminated (spec ex. 493).
  html_eq("[link](<foo\\>)\n", "<p>[link](&lt;foo&gt;)</p>");
  // Titles decode entities AND backslash escapes (spec ex. 506).
  html_eq(
    "[link](/url \"title \\\"&quot;\")\n",
    "<p><a href=\"/url\" title=\"title &quot;&quot;\">link</a></p>",
  );
}

#[test]
fn link_reference_definitions() {
  // A def may follow a heading (a heading is not lazy paragraph text) and the
  // heading resolves the shortcut (spec ex. 214).
  html_eq(
    "# [Foo]\n[foo]: /url\n> bar\n",
    "<h1><a href=\"/url\">Foo</a></h1>\n<blockquote>\n<p>bar</p>\n</blockquote>",
  );
  // A label may wrap across lines (spec ex. 541).
  html_eq(
    "[Foo\n  bar]: /url\n\n[Baz][Foo bar]\n",
    "<p><a href=\"/url\">Baz</a></p>",
  );
}

#[test]
fn entity_references_at_line_edges() {
  // A decoded entity that is whitespace must survive at the line start/end
  // (spec ex. 25/40).
  html_eq("&#9;foo\n", "<p>\tfoo</p>");
  html_eq("&nbsp; x\n", "<p>\u{a0} x</p>");
}

#[test]
fn html_block_type1_passes_through_tagfilter() {
  // A recognized type-1 HTML block (script/style/pre/textarea) is literal and
  // is NOT escaped by the GFM tagfilter (spec ex. 176/178).
  html_eq(
    "<style>p{color:red;}</style>\n*foo*\n",
    "<style>p{color:red;}</style>\n<p><em>foo</em></p>",
  );
}
