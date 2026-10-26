//! Storage behind the search index: every body back to back in ONE buffer,
//! optionally `mlock`ed so the kernel cannot page it out.
//!
//! Why (2026-10-20, production): the node runs ~40 containers in 3.5 GB and
//! keeps ~2 GB in swap. The index (15k bodies, 73 MB) was a `HashMap` of one
//! heap `String` per body; left idle for a minute its pages went to swap, and
//! the next search faulted them back in one 4 KB page at a time — `slow search`
//! logged 6–37 s in the scan phase, while a warm scan was well under a second.
//! Halving the index (0.13.60) did not change the shape of that, because the
//! index was still allowed to leave RAM.
//!
//! So the bodies live in a single allocation that can be locked as a whole.
//! The lock is an operator switch (`MICA_SEARCH_INDEX_MLOCK`, on by default)
//! and needs `CAP_IPC_LOCK` in the container (compose `cap_add: [IPC_LOCK]`).
//! Without it `mlock` fails, the failure is reported once, and the index works
//! exactly as before — swappable — rather than refusing to start.
//!
//! Replaced bodies leave dead bytes behind; they are reclaimed whenever the
//! buffer has to grow, by copying only the live bodies into the new buffer, so
//! the dead share stays bounded by the growth headroom.

use std::{collections::HashMap, fmt};

use uuid::Uuid;

/// Spare room left after each grow, as a fraction of the live bytes: edits
/// append without reallocating until it runs out (~7 MB at production's 70 MB,
/// then one ~30 ms copy), and a locked buffer pins little beyond the bodies —
/// on a node already in swap, every locked megabyte is taken from someone.
const HEADROOM: f64 = 0.10;
/// Floor for that headroom, so a nearly empty index does not regrow per edit.
const MIN_HEADROOM: usize = 1 << 20;

/// Whether the arena's pages are pinned in RAM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockState {
  /// The operator turned it off (`MICA_SEARCH_INDEX_MLOCK=off`), or tests.
  Disabled,
  /// Every page of the current buffer is locked.
  Locked,
  /// Asked for, refused by the OS — stays swappable. Not retried until the
  /// process restarts: the cause (a missing capability, a memlock limit) does
  /// not change at runtime, and a warning per edit would bury the logs.
  Failed(String),
}

impl fmt::Display for LockState {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      LockState::Disabled => f.write_str("off"),
      LockState::Locked => f.write_str("locked"),
      LockState::Failed(why) => write!(f, "not locked ({why})"),
    }
  }
}

#[derive(Clone, Copy)]
struct Slot {
  start: usize,
  len: usize,
  /// Whether the body has an ASCII capital — see [`Arena::contains`].
  has_upper: bool,
}

/// One allocation, locked for as long as it lives (when locking worked).
struct Buffer {
  text: String,
  locked: bool,
}

impl Buffer {
  fn with_capacity(capacity: usize) -> Self {
    Buffer {
      text: String::with_capacity(capacity),
      locked: false,
    }
  }

  /// Pin every reserved page. Called only once the buffer is FILLED (see
  /// [`Arena::insert_all`]): locking forces the whole reservation resident at
  /// once, and doing that while the incoming rows were still alive doubled
  /// the peak — on a node already in swap that held the 0.13.62 api past its
  /// healthcheck. Returns the error instead of failing: an unlocked index
  /// still answers searches.
  fn lock(&mut self) -> Result<(), String> {
    os::lock(self.text.as_ptr(), self.text.capacity())?;
    self.locked = true;
    Ok(())
  }
}

impl Drop for Buffer {
  fn drop(&mut self) {
    // A freed allocation is not necessarily unmapped (the allocator may keep
    // and reuse it), so unlock explicitly rather than leaking locked pages.
    if self.locked {
      os::unlock(self.text.as_ptr(), self.text.capacity());
    }
  }
}

/// `document_id → body`, every body stored in one buffer.
pub(crate) struct Arena {
  buffer: Buffer,
  slots: HashMap<Uuid, Slot>,
  /// Bytes belonging to current bodies; the rest of `buffer.text` is dead.
  live: usize,
  lock: LockState,
}

impl Arena {
  pub(crate) fn new(lock: bool) -> Self {
    Arena {
      buffer: Buffer {
        text: String::new(),
        locked: false,
      },
      slots: HashMap::new(),
      live: 0,
      lock: if lock {
        LockState::Locked
      } else {
        LockState::Disabled
      },
    }
  }

  /// Insert or replace bodies. Room for the whole batch is made once, so the
  /// startup load of every body is one allocation (and one lock), not dozens.
  /// Rows are consumed one by one, each freed right after its copy, and the
  /// lock comes last — the peak is one corpus, not two.
  pub(crate) fn insert_all(&mut self, rows: Vec<(Uuid, String)>) {
    let incoming: usize = rows.iter().map(|(_, text)| text.len()).sum();
    // Retire the bodies this batch replaces BEFORE sizing the grow: they are
    // dead once their copies land, so the reservation must hold the surviving
    // live bodies plus the incoming batch — not the soon-dead old corpus
    // (issue #106). `remove` returns None for a repeated id, so a batch that
    // somehow named one document twice still retires it once.
    for (id, _) in &rows {
      if let Some(old) = self.slots.remove(id) {
        self.live -= old.len;
      }
    }
    if self.buffer.text.len() + incoming > self.buffer.text.capacity() {
      // Live bytes after this batch: the replaced bodies already left it, so
      // the copy below carries the survivors and drops every dead byte on the
      // way, exactly like the arena doc promises.
      self.regrow(self.live + incoming);
    }
    for (id, text) in rows {
      let start = self.buffer.text.len();
      self.buffer.text.push_str(&text);
      let has_upper = text.bytes().any(|b| b.is_ascii_uppercase());
      self.slots.insert(
        id,
        Slot {
          start,
          len: text.len(),
          has_upper,
        },
      );
      self.live += text.len();
    }
    if self.lock == LockState::Locked && !self.buffer.locked && self.buffer.text.capacity() > 0 {
      if let Err(why) = self.buffer.lock() {
        tracing::warn!(
          reason = %why,
          "search index could not be locked in memory; it stays swappable \
           (needs CAP_IPC_LOCK — compose `cap_add: [IPC_LOCK]` — or set MICA_SEARCH_INDEX_MLOCK=off)"
        );
        self.lock = LockState::Failed(why);
      }
    }
  }

  /// Move the live bodies into a fresh buffer sized for `need` plus headroom,
  /// dropping every dead byte on the way. The old buffer is unlocked and freed;
  /// the new one is locked by the caller once filled.
  fn regrow(&mut self, need: usize) {
    let capacity = need + ((need as f64 * HEADROOM) as usize).max(MIN_HEADROOM);
    // Unlocked for now: `insert_all` locks it once it is filled.
    let mut next = Buffer::with_capacity(capacity);
    for slot in self.slots.values_mut() {
      let start = next.text.len();
      next
        .text
        .push_str(&self.buffer.text[slot.start..slot.start + slot.len]);
      slot.start = start;
    }
    self.buffer = next;
  }

  fn text(&self, slot: &Slot) -> &str {
    &self.buffer.text[slot.start..slot.start + slot.len]
  }

  /// Ids whose body contains `folded` (an ASCII-lowercased needle), ASCII-
  /// case-insensitively — what `ILIKE` under the C locale folds, so the scan
  /// agrees with the `v.name ILIKE` half still done in SQL.
  ///
  /// A plain `contains` is exact unless BOTH sides have ASCII letters in play:
  /// folding only rewrites `A`–`Z`, so a needle with no ASCII letter (every
  /// CJK query) can only match bytes the fold leaves alone.
  pub(crate) fn matching(&self, folded: &str) -> Vec<Uuid> {
    let needle_has_letters = folded.bytes().any(|b| b.is_ascii_lowercase());
    let needle = folded.as_bytes();
    self
      .slots
      .iter()
      .filter(|(_, slot)| {
        let text = self.text(slot);
        if !slot.has_upper || !needle_has_letters {
          return text.contains(folded);
        }
        let hay = text.as_bytes();
        hay.len() >= needle.len()
          && hay
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
      })
      .map(|(id, _)| *id)
      .collect()
  }

  pub(crate) fn body(&self, id: &Uuid) -> Option<&str> {
    self.slots.get(id).map(|slot| self.text(slot))
  }

  pub(crate) fn documents(&self) -> usize {
    self.slots.len()
  }

  /// `(live bytes, reserved bytes, lock state)` — for the startup log.
  pub(crate) fn footprint(&self) -> (usize, usize, LockState) {
    (self.live, self.buffer.text.capacity(), self.lock.clone())
  }
}

#[cfg(unix)]
mod os {
  pub(super) fn lock(ptr: *const u8, len: usize) -> Result<(), String> {
    // SAFETY: `ptr..ptr+len` is exactly one live allocation owned by the
    // calling `Buffer`; mlock only changes its residency, never its contents.
    if unsafe { libc::mlock(ptr.cast(), len) } == 0 {
      Ok(())
    } else {
      Err(std::io::Error::last_os_error().to_string())
    }
  }

  pub(super) fn unlock(ptr: *const u8, len: usize) {
    // SAFETY: same range `lock` succeeded on, still owned by the `Buffer`
    // being dropped. A failure here has nothing left to undo.
    unsafe {
      libc::munlock(ptr.cast(), len);
    }
  }
}

#[cfg(not(unix))]
mod os {
  pub(super) fn lock(_: *const u8, _: usize) -> Result<(), String> {
    Err("memory locking is only implemented on unix".into())
  }

  pub(super) fn unlock(_: *const u8, _: usize) {}
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The arena must answer exactly what a plain map of strings would, through
  /// any sequence of inserts and replacements — including the regrows (and
  /// compactions) a long run of edits forces.
  #[test]
  fn behaves_like_a_map_through_replacements_and_regrows() {
    let ids: Vec<Uuid> = (0..40).map(|_| Uuid::new_v4()).collect();
    let mut arena = Arena::new(false);
    let mut reference: HashMap<Uuid, String> = HashMap::new();
    // Deterministic pseudo-random sequence; no rand crate for a test.
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
      seed ^= seed << 13;
      seed ^= seed >> 7;
      seed ^= seed << 17;
      seed
    };
    let words = [
      "部署",
      "Docker",
      "docker",
      "备份",
      "Mica",
      "search",
      "会议纪要",
      "x",
    ];
    let mut max_live = 0usize;
    for round in 0..300 {
      let batch: Vec<(Uuid, String)> = (0..(next() % 5 + 1))
        .map(|_| {
          let id = ids[(next() % ids.len() as u64) as usize];
          let len = (next() % 400) as usize;
          let text: String = (0..len)
            .map(|_| words[(next() % words.len() as u64) as usize])
            .collect::<Vec<_>>()
            .join(" ");
          (id, format!("{round} {text}"))
        })
        .collect();
      for (id, text) in &batch {
        reference.insert(*id, text.clone());
      }
      arena.insert_all(batch);

      assert_eq!(arena.documents(), reference.len());
      for (id, text) in &reference {
        assert_eq!(arena.body(id), Some(text.as_str()));
      }
      for needle in ["docker", "部署", "mica", "会议", "zz"] {
        let mut got = arena.matching(needle);
        let mut want: Vec<Uuid> = reference
          .iter()
          .filter(|(_, t)| t.to_ascii_lowercase().contains(needle))
          .map(|(id, _)| *id)
          .collect();
        got.sort();
        want.sort();
        assert_eq!(got, want, "round {round} needle {needle:?}");
      }
      // Edits must not grow memory without bound: dead bytes are dropped at
      // every regrow, so what is reserved tracks the largest live set seen
      // (plus headroom and one batch), however many replacements came before.
      let (live, reserved, _) = arena.footprint();
      max_live = max_live.max(live);
      let bound = max_live + ((max_live as f64 * HEADROOM) as usize).max(MIN_HEADROOM) + 64 * 1024;
      assert!(
        reserved <= bound,
        "round {round}: reserved {reserved} > bound {bound} (live {live})"
      );
    }
  }

  #[test]
  fn lock_state_is_reported_not_fatal() {
    let mut off = Arena::new(false);
    off.insert_all(vec![(Uuid::new_v4(), "body".into())]);
    assert_eq!(off.footprint().2, LockState::Disabled);

    // Whether this host lets the test process lock 1 MB depends on its
    // memlock limit; either answer is fine, the index must work regardless.
    let mut on = Arena::new(true);
    let id = Uuid::new_v4();
    on.insert_all(vec![(id, "Deploy with Docker".into())]);
    assert!(matches!(
      on.footprint().2,
      LockState::Locked | LockState::Failed(_)
    ));
    assert_eq!(on.matching("docker"), vec![id]);
  }
}
