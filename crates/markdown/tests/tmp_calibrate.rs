//! THROWAWAY calibration for the /ai/edits/preview input caps (issue #44).
//! Not committed — deleted after running.

fn ms(src: &str) -> u128 {
  let started = std::time::Instant::now();
  let payload = mica_markdown::import_markdown_fragment(src, "root");
  let parse = started.elapsed().as_millis();
  let started = std::time::Instant::now();
  let _ = mica_markdown::export_markdown(&payload).unwrap();
  let export = started.elapsed().as_millis();
  eprintln!(
    "len={:>9} parse={}ms export={}ms blocks={}",
    src.len(),
    parse,
    export,
    payload.blocks.len()
  );
  parse + export
}

#[test]
fn calibrate() {
  eprintln!("--- benign paragraphs ---");
  for n in [64, 256, 1024, 2048] {
    let src = format!(
      "{}\n\n",
      "some ordinary paragraph text with a **bold** bit and `code`.".repeat(n / 60)
    );
    ms(&src);
  }
  eprintln!("--- adversarial unclosed \\( ---");
  for n in [64, 256, 1024, 2048] {
    let src = "\\(x ".repeat(n * 1024 / 4);
    ms(&src);
  }
  eprintln!("--- adversarial _a email ---");
  for n in [64, 256, 1024] {
    let src = "_a".repeat(n * 1024 / 2);
    ms(&src);
  }
  eprintln!("--- adversarial *a* html span ---");
  for n in [64, 256, 1024] {
    let src = "*a* ".repeat(n * 1024 / 4);
    ms(&src);
  }
}
