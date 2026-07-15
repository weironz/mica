//! CJK-friendly emphasis (markdown-cjk-friendly amendment). Plain CommonMark
//! treats CJK punctuation as "punctuation" for the flanking rule, so a `**`
//! that closes right after a full-width period `。` (or opens right after `：`)
//! fails to pair — extremely common in Chinese/Japanese/Korean prose. The
//! amendment relaxes flanking across CJK boundaries; ASCII stays untouched
//! (the scoreboard is the guard for that). Mirrors the Dart `_flanking`.

use mica_markdown::{export_html, import_markdown};

fn html_eq(md: &str, expected: &str) {
  let got = export_html(&import_markdown(md, "root")).unwrap_or_default();
  assert_eq!(got.trim_end(), expected, "markdown: {md:?}");
}

#[test]
fn strong_closes_after_cjk_period() {
  // The reported case: `。` before the closing `**`, a CJK letter after.
  html_eq(
    "结论：**不能只靠日志。**它是入口\n",
    "<p>结论：<strong>不能只靠日志。</strong>它是入口</p>",
  );
  // Japanese / Korean forms from the reference implementation.
  html_eq("**この文章。**次の文\n", "<p><strong>この文章。</strong>次の文</p>");
  html_eq(
    "**이 별표。**이 문장\n",
    "<p><strong>이 별표。</strong>이 문장</p>",
  );
}

#[test]
fn emphasis_between_cjk_and_punctuation() {
  // Italic bounded by CJK punctuation on both ends.
  html_eq("这是*重点*。\n", "<p>这是<em>重点</em>。</p>");
  // Underscore emphasis works across CJK boundaries too.
  html_eq("下划线_强调_文字\n", "<p>下划线<em>强调</em>文字</p>");
  // A comma (full-width) before the closer.
  html_eq("**加粗,**后面\n", "<p><strong>加粗,</strong>后面</p>");
}

#[test]
fn ascii_emphasis_is_unchanged() {
  // The strict ASCII behavior must not shift (the scoreboard covers the whole
  // suite; these pin the intraword cases the amendment could have disturbed).
  html_eq("**hello world**\n", "<p><strong>hello world</strong></p>");
  html_eq("foo**bar**baz\n", "<p>foo<strong>bar</strong>baz</p>");
  // Intraword underscore stays literal (snake_case is not emphasis).
  html_eq("foo_bar_baz\n", "<p>foo_bar_baz</p>");
  // A `*` hugged by ASCII punctuation stays literal (spec flanking).
  html_eq("a**\"foo\"**\n", "<p>a**&quot;foo&quot;**</p>");
}

#[test]
fn round_trip_cjk_bold() {
  // The invariant that was broken: export then re-import must be stable.
  let md = "结论:**不能只靠日志。**它是入口\n";
  let doc = import_markdown(md, "root");
  let html = export_html(&doc).unwrap_or_default();
  assert!(
    html.contains("<strong>不能只靠日志。</strong>"),
    "round-trip lost the CJK bold: {html}"
  );
}
