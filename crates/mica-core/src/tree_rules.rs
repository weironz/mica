//! Page-tree rules that the server (Postgres) and the local store (SQLite)
//! must agree on, as pure functions: no database, no I/O.
//!
//! Each store still reads its own facts (one indexed lookup, one recursive
//! walk) and does its own writes; what lives here is the DECISION, so the two
//! cannot drift apart the way `restore_view_subtree` once did from the
//! server's `restore_views_batch` (docs/dev/lessons.md §2, "同一条规则有两份").
//! The rules that are a single SQL statement on each side (trash a subtree,
//! restore one trash batch, lift an orphaned root) stay in SQL; the shared
//! scenario file `contracts/tree-rules.json` is replayed against both stores
//! instead, so a change to one side that the other lacks fails a test.

use std::fmt;

/// What the store found when it looked up a would-be parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentLookup {
  /// A live folder: the only kind of container.
  Folder,
  /// A live view that is not a folder (a page).
  NotFolder,
  /// No live view by that id where the store looked.
  Missing,
}

/// How a store treats a parent it cannot see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingParent {
  /// The server is the authority on its tree: an unknown parent is a 404.
  Refuse,
  /// The local store accepts it: rows can arrive child-first during a sync,
  /// and the server re-checks cloud rows on the way up.
  Accept,
}

/// Why a view may not go under a parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentRefusal {
  /// The view was named as its own parent.
  SelfParent,
  /// The parent is a page; pages cannot contain pages.
  NotAFolder,
  /// The parent does not exist (or is in the recycle bin) here.
  NotFound,
  /// The parent is inside the view's own subtree: the move would close a loop.
  IntoOwnSubtree,
}

impl ParentRefusal {
  /// A stable machine code, as `contracts/tree-rules.json` names refusals.
  pub fn code(self) -> &'static str {
    match self {
      Self::SelfParent => "self_parent",
      Self::NotAFolder => "not_a_folder",
      Self::NotFound => "not_found",
      Self::IntoOwnSubtree => "into_own_subtree",
    }
  }

  /// Every refusal, for callers that map a reason back to its code.
  pub const ALL: [Self; 4] = [
    Self::SelfParent,
    Self::NotAFolder,
    Self::NotFound,
    Self::IntoOwnSubtree,
  ];

  /// The reason, worded once for both stores.
  pub fn reason(self) -> &'static str {
    match self {
      Self::SelfParent => "a view cannot be its own parent",
      Self::NotAFolder => "only a folder can hold pages",
      Self::NotFound => "the parent does not exist or is in the recycle bin",
      Self::IntoOwnSubtree => "a view cannot move into its own subtree",
    }
  }
}

impl fmt::Display for ParentRefusal {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(self.reason())
  }
}

/// The checks that need only the parent's own row, in the order both stores
/// apply them. A store that gets `Ok` then runs its subtree walk and refuses
/// with [`ParentRefusal::IntoOwnSubtree`] if the parent is a descendant: the
/// walk is the one expensive step, so it runs last.
pub fn judge_parent(
  is_self: bool,
  parent: ParentLookup,
  missing: MissingParent,
) -> Result<(), ParentRefusal> {
  if is_self {
    return Err(ParentRefusal::SelfParent);
  }
  match (parent, missing) {
    (ParentLookup::Folder, _) | (ParentLookup::Missing, MissingParent::Accept) => Ok(()),
    (ParentLookup::NotFolder, _) => Err(ParentRefusal::NotAFolder),
    (ParentLookup::Missing, MissingParent::Refuse) => Err(ParentRefusal::NotFound),
  }
}

/// `base` if free among `siblings`, else `base 2`, `base 3`, … The number is
/// locale-neutral; the caller supplies the localized base ("X 副本" / "X copy").
/// A page cloned offline and one cloned in the cloud settle on the same name.
pub fn dedup_sibling_name(base: &str, siblings: &[String]) -> String {
  if !siblings.iter().any(|s| s == base) {
    return base.to_string();
  }
  (2..)
    .map(|n| format!("{base} {n}"))
    .find(|c| !siblings.iter().any(|s| s == c))
    .expect("an unused suffix always exists")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_folders_hold_children() {
    for missing in [MissingParent::Refuse, MissingParent::Accept] {
      assert_eq!(judge_parent(false, ParentLookup::Folder, missing), Ok(()));
      assert_eq!(
        judge_parent(false, ParentLookup::NotFolder, missing),
        Err(ParentRefusal::NotAFolder)
      );
    }
  }

  #[test]
  fn an_unseen_parent_is_the_one_policy_difference() {
    assert_eq!(
      judge_parent(false, ParentLookup::Missing, MissingParent::Refuse),
      Err(ParentRefusal::NotFound)
    );
    assert_eq!(
      judge_parent(false, ParentLookup::Missing, MissingParent::Accept),
      Ok(())
    );
  }

  #[test]
  fn self_parent_is_refused_before_looking_at_the_parent() {
    assert_eq!(
      judge_parent(true, ParentLookup::Folder, MissingParent::Accept),
      Err(ParentRefusal::SelfParent)
    );
  }

  #[test]
  fn dedup_only_numbers_on_collision() {
    assert_eq!(dedup_sibling_name("日志方案 副本", &[]), "日志方案 副本");
    let sibs = vec!["Notes".to_string()];
    assert_eq!(dedup_sibling_name("Notes", &sibs), "Notes 2");
    let sibs = vec!["Notes".into(), "Notes 2".into(), "Notes 3".into()];
    assert_eq!(dedup_sibling_name("Notes", &sibs), "Notes 4");
    let sibs = vec!["Notes".into(), "Notes 3".into()];
    assert_eq!(dedup_sibling_name("Notes", &sibs), "Notes 2");
    // Unrelated siblings never force a suffix.
    assert_eq!(
      dedup_sibling_name("X 副本", &["Y".into(), "Z".into()]),
      "X 副本"
    );
  }
}
