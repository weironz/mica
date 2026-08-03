//! What counts as a password this server will accept.
//!
//! The only rule used to be `len() >= 8`, which accepts `password`, `12345678`
//! and the user's own email local part. Online guessing is already throttled
//! (per-IP bucket + the global Argon2 gate in `rate_limit.rs`), so this is not
//! about brute force — it is about the two guesses that do not need many tries:
//! the password everybody picks, and the one made out of the account's own name.
//!
//! **Shaped after NIST SP 800-63B**, which is unusually specific about what NOT
//! to do: no composition rules (upper/digit/symbol), no forced rotation, no
//! arbitrary length inflation. Those push people toward `Password1!` and a
//! sticky note. What it does ask for is a check against commonly-used and
//! context-specific values — which is what this does.
//!
//! **What it deliberately does not do:** check the password against a breach
//! corpus. The good version of that is Have I Been Pwned's k-anonymity API, and
//! it costs an outbound HTTPS call on every sign-up and password change — a
//! third party in the registration path of a self-hosted note app, failing
//! closed (nobody can register while they are down) or open (the check silently
//! stops happening). Neither is worth it here. Recorded rather than forgotten.

/// Why a password was refused. Each maps to a message a user can act on —
/// "invalid password" tells someone to go try another bad one.
#[derive(Debug, PartialEq, Eq)]
pub enum Weakness {
  TooShort,
  TooCommon,
  NoVariety,
  LooksLikeYou,
}

impl Weakness {
  pub fn message(&self) -> &'static str {
    match self {
      Weakness::TooShort => "password must be at least 8 characters",
      Weakness::TooCommon => {
        "that password is one of the most commonly used ones — pick another"
      }
      Weakness::NoVariety => {
        "that password is a single repeated character or a straight sequence"
      }
      Weakness::LooksLikeYou => {
        "password must not be built from your email address or display name"
      }
    }
  }
}

/// Eight, and not more.
///
/// NIST's floor, and raising it is the composition-rule mistake wearing a
/// different hat: it does not stop `Password123`, and it does push people toward
/// patterns. Length is not what is being checked here — predictability is.
const MIN_LENGTH: usize = 8;

/// The passwords that top every published breach analysis, plus the shapes this
/// codebase's own placeholders would produce.
///
/// **Every entry is at least [`MIN_LENGTH`]**, enforced by a test. Shorter ones
/// (`qwerty`, `letmein`, `monkey`) were in here at first and were dead data: the
/// length check runs first, so they could never be reached. A list that looks
/// like it covers something it cannot is worse than a shorter honest one.
///
/// Deliberately short and hand-picked rather than a bundled top-10k list: the
/// long tail of such a list is already caught by [`is_low_variety`] or is rare
/// enough not to matter, and a 100KB data file in the binary would need a
/// provenance story (where from, what licence, refreshed by whom) that nothing
/// here is prepared to keep up.
const COMMON: &[&str] = &[
  "password",
  "passw0rd",
  "password1",
  "password123",
  "12345678",
  "123456789",
  "1234567890",
  "qwerty123",
  "qwertyui",
  "qwertyuiop",
  "abc12345",
  "iloveyou",
  "sunshine",
  "princess",
  "football",
  "baseball",
  "welcome1",
  "admin123",
  "administrator",
  "superman",
  "trustno1",
  "starwars",
  "whatever",
  "computer",
  "internet",
  "facebook",
  "asdfghjk",
  "asdfghjkl",
  "1qaz2wsx",
  "1q2w3e4r",
  "qazwsxedc",
  "changeme",
  "change-me",
  "secret123",
  "test1234",
  "abcd1234",
  "a1b2c3d4",
  "11111111",
  "00000000",
  "88888888",
  "66666666",
  "123123123",
  "woaini1314",
  "5201314520",
];

/// A password with no information in it: one character over and over, or a
/// straight run up or down the alphabet, the digits, or a keyboard row.
///
/// Catches the long tail a fixed list cannot — `aaaaaaaa`, `23456789`,
/// `hgfedcba`, `poiuytre` — without pretending to be a strength meter.
fn is_low_variety(password: &str) -> bool {
  let chars: Vec<char> = password.chars().collect();
  if chars.len() < 2 {
    return true;
  }

  if chars.iter().all(|c| *c == chars[0]) {
    return true;
  }

  // A run in either direction across the whole string. Codepoint adjacency
  // covers digits and latin letters; keyboard rows need their own pass, since
  // `qwerty` is adjacent on a keyboard and nowhere in Unicode.
  let ascending = chars
    .windows(2)
    .all(|pair| pair[1] as u32 == pair[0] as u32 + 1);
  let descending = chars
    .windows(2)
    .all(|pair| pair[0] as u32 == pair[1] as u32 + 1);
  if ascending || descending {
    return true;
  }

  const KEYBOARD_ROWS: &[&str] = &["qwertyuiop", "asdfghjkl", "zxcvbnm", "1234567890"];
  let lower = password.to_lowercase();
  let reversed: String = lower.chars().rev().collect();
  KEYBOARD_ROWS
    .iter()
    .any(|row| row.contains(&lower) || row.contains(&reversed))
}

/// Does the password lean on something an attacker already knows about the
/// account?
///
/// Both directions, because both happen: `alice` inside `alice2024`, and a whole
/// password that is a substring of the display name. Segments under 4 characters
/// are ignored — a surname like `wu` would otherwise ban every password with
/// those two letters anywhere in it, and a rule that refuses good passwords
/// teaches people to work around it.
fn echoes_identity(password: &str, identifiers: &[&str]) -> bool {
  let lower = password.to_lowercase();
  identifiers
    .iter()
    .flat_map(|value| {
      let value = value.to_lowercase();
      // The local part carries the name; the domain is shared by everyone on a
      // self-hosted instance, and banning it would refuse the same password for
      // every user at once.
      let local = value.split('@').next().unwrap_or(&value).to_string();
      local
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_string)
        .collect::<Vec<_>>()
    })
    .filter(|segment| segment.chars().count() >= 4)
    .any(|segment| lower.contains(&segment) || segment.contains(&lower))
}

/// The one gate. Both sign-up and change-password go through here — they used to
/// carry a `len() < 8` check each, which is the same rule written twice and the
/// reason one of them could have drifted with nothing failing.
pub fn check(password: &str, identifiers: &[&str]) -> Result<(), Weakness> {
  if password.chars().count() < MIN_LENGTH {
    return Err(Weakness::TooShort);
  }
  let lower = password.to_lowercase();
  if COMMON.contains(&lower.as_str()) {
    return Err(Weakness::TooCommon);
  }
  if is_low_variety(password) {
    return Err(Weakness::NoVariety);
  }
  if echoes_identity(password, identifiers) {
    return Err(Weakness::LooksLikeYou);
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_reasonable_password_is_accepted() {
    // No composition rules: all-lowercase with no digits is fine when it is not
    // predictable. Refusing this is how you end up with `Password1!`.
    assert_eq!(check("correcthorsebatterystaple", &[]), Ok(()));
    assert_eq!(check("mica-notes-2026", &["wei@example.com"]), Ok(()));
  }

  #[test]
  fn the_floor_is_still_eight() {
    assert_eq!(check("short12", &[]), Err(Weakness::TooShort));
    assert_eq!(check("eightchr", &[]), Ok(()));
  }

  /// Counted in CHARACTERS, not bytes. `len()` on a UTF-8 string calls a
  /// three-ideograph password "nine characters" and waves it through.
  #[test]
  fn length_is_counted_in_characters() {
    assert_eq!(
      check("密码密", &[]),
      Err(Weakness::TooShort),
      "3 chars, 9 bytes"
    );
    assert_eq!(check("密码密码密码密码", &[]), Ok(()), "8 chars");
  }

  #[test]
  fn the_obvious_ones_are_refused() {
    for bad in ["password", "PASSWORD", "Qwerty123", "welcome1", "iloveyou"] {
      assert_eq!(check(bad, &[]), Err(Weakness::TooCommon), "{bad}");
    }
  }

  #[test]
  fn a_password_with_no_information_in_it_is_refused() {
    for bad in ["aaaaaaaa", "12345678", "87654321", "abcdefgh", "qwertyui"] {
      assert!(
        matches!(
          check(bad, &[]),
          Err(Weakness::NoVariety | Weakness::TooCommon)
        ),
        "{bad}"
      );
    }
  }

  /// The guess that does not need many tries. Both directions, because both
  /// happen in the wild.
  #[test]
  fn a_password_built_from_the_account_is_refused() {
    assert_eq!(
      check("zhangwei2026", &["zhangwei@example.com"]),
      Err(Weakness::LooksLikeYou),
      "the email local part, padded with a year"
    );
    assert_eq!(
      check("wei.zhang", &["wei.zhang@example.com", "Wei Zhang"]),
      Err(Weakness::LooksLikeYou)
    );
    assert_eq!(
      check("zhangweiwei", &["Zhang Wei"]),
      Err(Weakness::LooksLikeYou),
      "display name, not just the email"
    );
  }

  /// A shorter-than-minimum entry can never be reached — the length check runs
  /// first — so it would sit in the list looking like coverage it does not
  /// provide. This is what caught the first draft, which had `qwerty` in it.
  #[test]
  fn no_entry_in_the_list_is_unreachable() {
    for entry in COMMON {
      assert!(
        entry.chars().count() >= MIN_LENGTH,
        "{entry:?} is shorter than the minimum length, so it is dead data"
      );
    }
  }

  /// A short segment must not ban half the dictionary.
  #[test]
  fn short_name_segments_do_not_ban_everything() {
    assert_eq!(check("powerful-notes", &["wu@example.com"]), Ok(()));
    assert_eq!(check("subterranean", &["su@example.com"]), Ok(()));
  }

  /// The domain is shared by everyone on a self-hosted instance.
  #[test]
  fn the_email_domain_is_not_part_of_the_identity() {
    assert_eq!(check("example-notebook", &["wei@example.com"]), Ok(()));
  }
}
