//! Reproduce the S2 native/Web feature-unification difference without exports.
fn main() {
  for (name, source) in [
    ("math", "$x$"),
    ("mermaid", "```mermaid\ngraph TD; A-->B\n```"),
  ] {
    let blocks = mica_md::wire::parse_body_json(source, false).unwrap();
    for profile in ["export", "clipboard"] {
      let html = mica_md::wire::render_html_body_json(&blocks, "{}", profile).unwrap();
      println!("{name}.{profile}\t{}\t{html:?}", html.len());
    }
  }
}
