//! Block-id generation, injectable so the engine needs no randomness.

/// How a parse assigns ids to the blocks it creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockIds {
  /// `block_<uuid-v4-simple>` — the historical format. Requires feature
  /// `uuid` (on by default).
  #[cfg(feature = "uuid")]
  Uuid,
  /// `<prefix><n>`, `n` from 0 in block order. Deterministic; used by wasm and
  /// tests, and needs no random number generator.
  Sequential {
    /// Prepended to each sequential number.
    prefix: String,
  },
}

impl BlockIds {
  /// A deterministic id for the block at position `n`.
  ///
  /// Only the `Uuid` variant calls this (the sequential path in `assign_ids`
  /// formats ids inline), so the whole function belongs to the feature.
  #[cfg(feature = "uuid")]
  pub(crate) fn id_for(&self, n: usize) -> String {
    match self {
      BlockIds::Uuid => format!("block_{}", uuid::Uuid::new_v4().simple()),
      // Unreachable in practice (see the doc above), kept for exhaustive match.
      #[allow(unreachable_patterns)]
      BlockIds::Sequential { prefix } => format!("{prefix}{n}"),
    }
  }
}
