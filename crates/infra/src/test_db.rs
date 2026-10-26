//! The one gate every Postgres-backed test goes through.
//!
//! A DB test without a database returns early, and a test that returns early
//! reports as PASSED. This used to be a hand-copied `let Ok(url) = env::var(..)
//! else { assert!(CI unset) }` in 30+ places, in three wordings; one missed
//! copy is a suite that silently stops testing anything. Now there is one
//! place to get wrong.

/// `DATABASE_URL` for a DB-backed test, or `None` to skip it.
///
/// Skipping is only allowed on a developer machine. Under `CI`, or when the
/// local gate sets `MICA_REQUIRE_DB` (`scripts/local-ci.sh`), a missing URL is
/// a panic. Blank counts as unset: compose's `${VAR:-}` hands a process `""`,
/// not an absent variable.
pub fn test_database_url() -> Option<String> {
  resolve(
    std::env::var("DATABASE_URL").ok(),
    required(std::env::var("CI").ok()) || required(std::env::var("MICA_REQUIRE_DB").ok()),
  )
}

fn required(flag: Option<String>) -> bool {
  flag.is_some_and(|v| !matches!(v.trim(), "" | "0" | "false"))
}

fn resolve(url: Option<String>, required: bool) -> Option<String> {
  match url.filter(|u| !u.trim().is_empty()) {
    Some(url) => Some(url),
    None if required => panic!(
      "DATABASE_URL is unset but a database is required (CI / MICA_REQUIRE_DB): \
       DB-backed tests must not skip and report passed"
    ),
    None => {
      eprintln!("DATABASE_URL unset: skipping a Postgres-backed test");
      None
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_set_url_is_used() {
    assert_eq!(
      resolve(Some("postgres://x".into()), true).as_deref(),
      Some("postgres://x")
    );
  }

  #[test]
  fn unset_or_blank_skips_only_when_not_required() {
    assert_eq!(resolve(None, false), None);
    assert_eq!(resolve(Some("  ".into()), false), None);
  }

  #[test]
  #[should_panic(expected = "database is required")]
  fn blank_url_panics_when_required() {
    resolve(Some(String::new()), true);
  }

  #[test]
  fn required_flag_values() {
    assert!(required(Some("true".into())));
    assert!(required(Some("1".into())));
    assert!(!required(Some("".into())));
    assert!(!required(Some("0".into())));
    assert!(!required(None));
  }
}
