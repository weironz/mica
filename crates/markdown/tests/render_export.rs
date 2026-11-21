//! Rich-export rendering (feature `render`): math → SVG, mermaid → SVG.
#![cfg(feature = "render")]

use mica_markdown::export_html_document;
use mica_md::compat::import_markdown;

#[test]
fn math_and_mermaid_become_svg() {
  let md = "Inline $\\eta = \\frac{N-1}{N}$ text.\n\n\
              $$E = mc^2$$\n\n\
              ```mermaid\nflowchart LR\n  A[Start] --> B{Go}\n  B -->|Yes| C[世界]\n```\n";
  let snap = import_markdown(md, "root");
  let html = export_html_document(&snap, "render test", 800).unwrap();

  // LaTeX rendered to self-contained SVG (RaTeX), inline + block.
  assert!(
    html.contains("class=\"math-inline\""),
    "inline math should be SVG:\n{html}"
  );
  assert!(
    html.contains("class=\"math-block\""),
    "block math should be SVG"
  );
  // Mermaid rendered to a self-contained inline SVG, not raw source.
  assert!(
    html.contains("<div class=\"mermaid\"><svg"),
    "mermaid should be inline SVG:\n{html}"
  );
  assert!(
    !html.contains("language-mermaid"),
    "raw mermaid code block should be gone:\n{html}"
  );
}

#[test]
fn katex_grade_coverage_le_and_text() {
  // These exact constructs errored under the old latex2mathml path; RaTeX (KaTeX
  // parity) must render them without leaking any "PARSE ERROR" into the output.
  let md = "$0.1 \\le \\epsilon \\le 0.15$ and $B_{\\text{phy}} = 100 \\text{GB/s}$\n";
  let snap = import_markdown(md, "root");
  let html = export_html_document(&snap, "t", 800).unwrap();
  assert!(
    html.contains("class=\"math-inline\""),
    "should render as SVG:\n{html}"
  );
  assert!(
    !html.to_lowercase().contains("parse error") && !html.contains("Undefined("),
    "no parse errors should leak into output:\n{html}"
  );
}

#[test]
fn broken_mermaid_falls_back_to_code_block() {
  // A syntax error must not lose the source — it degrades to a code block.
  let md = "```mermaid\nthis is not valid mermaid @#$%\n```\n";
  let snap = import_markdown(md, "root");
  let html = export_html_document(&snap, "t", 800).unwrap();
  assert!(
    html.contains("<pre><code"),
    "invalid mermaid should keep its source:\n{html}"
  );
}

#[test]
fn mica_board_becomes_inline_svg() {
  let md = "```mica-board\n{\"version\":1,\"title\":\"测试画板\",\"mode\":\"whiteboard\",\"nodes\":[{\"id\":\"n1\",\"kind\":\"rounded\",\"x\":10,\"y\":10,\"w\":120,\"h\":48,\"text\":\"架构节点\"}],\"edges\":[],\"strokes\":[]}\n```\n";
  let snap = import_markdown(md, "root");
  let html = export_html_document(&snap, "board test", 800).unwrap();
  assert!(
    html.contains("<div class=\"mica-board\"><svg"),
    "mica-board should render as inline SVG:\n{html}"
  );
  assert!(html.contains("架构节点"));
  assert!(html.contains(".mica-board svg"));
}
