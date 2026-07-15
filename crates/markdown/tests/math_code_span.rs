use mica_markdown::import_markdown;

/// Regression: a math opener must not scan through a code span. CommonMark
/// 0.31.2 §6.1 gives code spans higher precedence than every inline construct
/// but HTML tags and autolinks, so the `$` inside one is literal and cannot
/// close an earlier `$`. Before the fix, `spent $5, config `$HOME` dir` came
/// back with a math run of "5, config `" — the code span's own backtick eaten
/// from the middle.
fn marks_of(src: &str, kind: &str) -> Vec<String> {
  let snap = import_markdown(src, "root");
  let mut out = vec![];
  for b in &snap.blocks {
    if b.text.is_empty() {
      continue;
    }
    let chars: Vec<char> = b.text.chars().collect();
    if let Some(ms) = b.data.get("marks").and_then(|m| m.as_array()) {
      for m in ms {
        if m.get("type").and_then(|t| t.as_str()) == Some(kind) {
          let s = m.get("start").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
          let e = m.get("end").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
          out.push(
            chars[s.min(chars.len())..e.min(chars.len())]
              .iter()
              .collect(),
          );
        }
      }
    }
  }
  out
}

#[test]
fn math_does_not_eat_through_a_code_span() {
  let src = "spent $5, config `$HOME` dir";
  assert!(marks_of(src, "math").is_empty(), "no math should be found");
  assert_eq!(marks_of(src, "code"), vec!["$HOME".to_string()]);
}

#[test]
fn code_span_content_survives_verbatim() {
  // The `$` and the backticks must both reach the text unharmed.
  let snap = import_markdown("run `echo $PATH` now", "root");
  let para = snap.blocks.iter().find(|b| !b.text.is_empty()).unwrap();
  assert_eq!(para.text, "run echo $PATH now");
  assert_eq!(marks_of("run `echo $PATH` now", "code"), vec!["echo $PATH"]);
  assert!(marks_of("run `echo $PATH` now", "math").is_empty());
}

#[test]
fn real_math_still_parses() {
  // The fix must not cost us the thing math is for.
  assert_eq!(
    marks_of(r"coef $\eta = 2 \times \frac{N-1}{N}$ ok", "math"),
    vec![r"\eta = 2 \times \frac{N-1}{N}".to_string()]
  );
}

#[test]
fn math_may_still_span_a_code_span_when_it_closes_after_one() {
  // Stepping over a code span must not mean dropping it: a `$` AFTER the span
  // still closes, and the backticks stay in the math verbatim (math content is
  // raw LaTeX — it is never re-parsed as markdown). Only the `$` INSIDE the
  // span is disqualified from closing.
  assert_eq!(
    marks_of("$a `x` b$ tail", "math"),
    vec!["a `x` b".to_string()]
  );
}

#[test]
fn an_unclosed_backtick_is_literal_and_does_not_block_math() {
  // An unclosed run is ordinary text per CommonMark — it must not make the
  // scan bail, or we would lose real math to a stray backtick.
  assert_eq!(marks_of("$a ` b$ tail", "math"), vec!["a ` b".to_string()]);
}
