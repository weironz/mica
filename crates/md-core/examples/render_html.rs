//! The two HTML profiles side by side: Export (share pages — sanitized,
//! rich) and Clipboard (what a paste flavor carries — source math and Mermaid
//! preserved as data attributes).

use mica_md::{HtmlOptions, HtmlProfile, ParseMode, parse_body, render_html_body};

fn main() {
  let markdown = concat!(
    "# Demo\n\n",
    "Inline math $E = mc^2$ and a <script>alert('no')</script> attempt.\n\n",
    "```mermaid\ngraph TD; A-->B;\n```\n"
  );
  let blocks = parse_body(markdown, ParseMode::Document);

  let export = render_html_body(&blocks, &HtmlOptions::default());
  let clipboard = render_html_body(
    &blocks,
    &HtmlOptions {
      profile: HtmlProfile::Clipboard,
      ..Default::default()
    },
  );

  println!("--- Export profile ---\n{export}\n");
  println!("--- Clipboard profile ---\n{clipboard}\n");
  println!("script survived export: {}", export.contains("<script"));
  println!("math source kept in clipboard: {}", clipboard.contains("data-mica-math"));
}