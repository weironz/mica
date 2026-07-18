//! Rich-export rendering (feature `render`): math → MathML, mermaid → SVG.
#![cfg(feature = "render")]

use mica_markdown::{export_html_document, import_markdown};

#[test]
fn math_becomes_mathml_and_mermaid_becomes_svg() {
    let md = "Inline $\\eta = \\frac{N-1}{N}$ text.\n\n\
              $$E = mc^2$$\n\n\
              ```mermaid\nflowchart LR\n  A[Start] --> B{Go}\n  B -->|Yes| C[世界]\n```\n";
    let snap = import_markdown(md, "root");
    let html = export_html_document(&snap, "render test", 800).unwrap();

    // LaTeX rendered to self-contained SVG (RaTeX), inline + block.
    assert!(html.contains("class=\"math-inline\""), "inline math should be SVG:\n{html}");
    assert!(html.contains("class=\"math-block\""), "block math should be SVG");
    // Mermaid rendered to a self-contained inline SVG, not raw source.
    assert!(html.contains("<div class=\"mermaid\"><svg"), "mermaid should be inline SVG:\n{html}");
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
    assert!(html.contains("class=\"math-inline\""), "should render as SVG:\n{html}");
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
    assert!(html.contains("<pre><code"), "invalid mermaid should keep its source:\n{html}");
}
