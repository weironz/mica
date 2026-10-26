//! Guard against LaTeX mangled by an under-escaped JSON string.
//!
//! The markdown engine is the right home for this (issue #83): the mangling is a
//! property of the markdown itself, every server write path that accepts authored
//! markdown needs the check, and the math-run rule must match the parser's real
//! rules (`math_run_spans`) or the guard drifts from what the document will render.
//!
//! `\times` under-escaped in JSON is not an error — `\t` is a legal escape — so
//! it decodes silently to TAB + "imes" and the formula is destroyed with no
//! diagnostic anywhere. Mica stores faithfully what it is given, and a correctly
//! escaped `\\times` round-trips fine; the damage comes from a caller hand-writing
//! JSON escapes. Refusing with a precise message lets the caller retry correctly;
//! storing it loses the formula forever.

use mica_md::math_run_spans;

/// Control characters a JSON unescape produces from a LaTeX command, paired with
/// the escape that made them, and an example of the command each one eats.
///
/// These four are safe to condemn: none has a legitimate reason to appear inside
/// a formula. `\n` and `\r` collide the same way (`\nabla`, `\rho`) but are
/// deliberately absent — a real newline is ordinary markdown and we cannot tell
/// the two apart. Their damage is at least visible: a newline breaks the math
/// run outright, so the page shows literal `$…$` rather than quietly wrong math.
pub const JSON_ESCAPE_COLLISIONS: [(char, &str, &str); 4] = [
  ('\u{0009}', r"\t", r"\times"),
  ('\u{000C}', r"\f", r"\frac"),
  ('\u{000B}', r"\v", r"\vec"),
  ('\u{0008}', r"\b", r"\beta"),
];

/// The JSON escape and example LaTeX command whose control character found in
/// [markdown] proves it was mangled, or `None` when the markdown is clean.
///
/// Two rules, both chosen to be false-positive-free rather than thorough:
///  - FF / VT / BS anywhere: no legitimate markdown contains them.
///  - TAB *inside a math run only*: a tab is ordinary indentation elsewhere, but
///    inside `$…$` it is meaningless to LaTeX and means `\t…` was eaten.
pub fn mangled_latex_collision(markdown: &str) -> Option<(&'static str, &'static str)> {
  JSON_ESCAPE_COLLISIONS
    .iter()
    .copied()
    .find(|(ch, _, _)| markdown.contains(*ch) && (*ch != '\u{0009}' || tab_inside_math(markdown)))
    .map(|(_, escape, example)| (escape, example))
}

fn tab_inside_math(markdown: &str) -> bool {
  let chars: Vec<char> = markdown.chars().collect();
  math_run_spans(markdown)
    .into_iter()
    .any(|(start, end)| chars[start..end].contains(&'\u{0009}'))
}

/// The human-facing refusal message: name the escape that decoded wrong and show
/// the exact fix (double every backslash), so the caller — usually a model — can
/// resend correctly instead of retrying the same corruption.
pub fn mangled_latex_message(escape: &str, example: &str) -> String {
  format!(
    "The markdown contains a raw control character where a LaTeX command should be: \
     `{escape}` was read as a JSON escape, so `{example}` arrived as a control character \
     instead of a command. The backslashes in the JSON string were under-escaped — in \
     JSON a literal backslash must be doubled. Send \"$\\\\times$\", not \"$\\times$\" \
     (the latter is a tab). Nothing was written; resend with every backslash doubled."
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  fn collision(markdown: &str) -> Option<(&'static str, &'static str)> {
    mangled_latex_collision(markdown)
  }

  /// Byte-for-byte what reached the database from a real write of
  /// `$\eta = 2 \times \frac{N-1}{N}$` with under-escaped backslashes. Spelled
  /// with \u{..} rather than Rust's own `\t`, because `"\times"` in Rust
  /// source IS tab+"imes" — the very confusion that caused the bug, and not
  /// something a test about it should re-enact.
  #[test]
  fn rejects_the_corruption_that_actually_shipped() {
    let mangled = concat!(
      "**A**: For $N$ nodes it is $\\eta = 2 ",
      "\u{0009}imes \u{000C}rac{N-1}{N}$, approaching $2$."
    );
    // Precondition: this really is the shape we saw — \eta intact, the
    // other two commands eaten down to a control char.
    assert!(mangled.contains(r"\eta"));
    assert!(!mangled.contains(r"\times") && !mangled.contains(r"\frac"));
    assert_eq!(collision(mangled), Some((r"\t", r"\times")));
  }

  #[test]
  fn rejects_each_collision_char() {
    // \f, \v, \b are impossible in real markdown — rejected anywhere.
    assert_eq!(collision("a \u{000C}rac b"), Some((r"\f", r"\frac")));
    assert_eq!(collision("a \u{000B}ec b"), Some((r"\v", r"\vec")));
    assert_eq!(collision("a \u{0008}eta b"), Some((r"\b", r"\beta")));
    // A tab only condemns itself inside a formula.
    assert_eq!(collision("$x \u{0009}imes y$"), Some((r"\t", r"\times")));
  }

  /// The guard must never fire on content someone legitimately wrote, or it
  /// blocks the very workflow it protects.
  #[test]
  fn accepts_legitimate_content() {
    // Correctly escaped LaTeX — the whole point.
    assert_eq!(collision(r"$\eta = 2 \times \frac{N-1}{N}$"), None);
    // A tab outside math is ordinary indentation / code.
    assert_eq!(
      collision("- item\n\u{0009}continued\n\n\u{0009}code();"),
      None
    );
    assert_eq!(collision("| a\u{0009}| b |\n|---|---|"), None);
    // Currency is not math (no valid closer), so its tabs stay innocent.
    assert_eq!(collision("costs $5\u{0009}and $10"), None);
    // Plain prose, newlines and CR are untouched.
    assert_eq!(collision("line one\r\nline two\n"), None);
  }

  /// A `$` inside a code span never opens a run (§6.1), so a tab after it is
  /// not "inside math" — proves the guard uses the parser's real rules.
  #[test]
  fn a_tab_after_a_dollar_in_a_code_span_is_not_math() {
    assert_eq!(collision("`$HOME`\tand `$PATH`\tare paths"), None);
  }

  #[test]
  fn the_message_tells_the_caller_how_to_fix_it() {
    let message = mangled_latex_message(r"\t", r"\times");
    assert!(message.contains("doubled"), "got: {message}");
    assert!(message.contains(r"\times"), "got: {message}");
  }
}
