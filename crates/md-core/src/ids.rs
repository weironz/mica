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
  pub(crate) fn id_for(&self, n: usize) -> String {
    match self {
      #[cfg(feature = "uuid")]
      BlockIds::Uuid => format!("block_{}", uuid::Uuid::new_v4().simple()),
      BlockIds::Sequential { prefix } => format!("{prefix}{n}"),
    }
  }
}
