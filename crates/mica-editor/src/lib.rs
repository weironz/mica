//! Mica L2 headless block editor kernel over `mica-core::MicaDoc` and `mica-md`.
//!
//! Pure Rust, zero UI/browser dependency, UTF-16 code-unit offsets throughout.

pub mod commands;
pub mod diff_ops;
pub mod history;
pub mod marks;
pub mod model;
pub mod paste;
pub mod rules;
pub mod selection;
pub mod session;
mod session_ops;
pub mod table;
pub mod utf16;

pub use commands::{
  AiBlockEdit, AiEditPreview, AiSnapshot, EditorCommand, EditorPatch, FindMatch, FindReplaceOp,
  PasteInput,
};
pub use diff_ops::{compute_move_blocks_target, diff_block_ops, diff_block_patches, in_order_ids};
pub use history::{EditorHistoryTracker, HistoryEndpoint, HistorySelection};
pub use marks::{
  EditorMark, apply_mark, apply_or_remove_mark, clip_marks, concat_marks, link_click_hit,
  marks_equal, marks_from_data, marks_to_json, math_run_at, math_run_ending_at,
  math_run_starting_at, normalize_marks, range_has_mark, remove_mark, shift_marks,
  snap_out_of_math_run, split_marks,
};
pub use model::{
  BlockPatch, BlockSpec, ClipboardPayload, DocOp, EditorBlock, EditorEffect, EditorError,
  MAX_NESTING_DEPTH, block_data_eq, heading_button_target,
};
pub use paste::{
  build_clipboard_payload, copy_language, html_nodes_to_markdown, looks_like_image_url,
  looks_like_url, normalize_line_endings, parse_paste_blocks, paste_merges_inline,
  pasted_formula_source, retag_mislabeled_fences, selection_clipboard_text, selection_md_blocks,
  single_line_paste_inline, single_line_paste_keeps_marks, unwrap_nested_fences, visible_ordinal,
};
pub use rules::{
  InputRuleMatch, detect_page_link_trigger, detect_slash_trigger, match_input_rule,
};
pub use selection::{
  ActiveMarks, DocPosition, DocSelection, InlineSelectionRange, NormalizedSelection,
  active_marks_for_selection, inline_selection_ranges, reconcile_selection, word_range_at,
};
pub use session::{EditorSession, find_all_matches};
pub use table::{TableCell, TableModel, TableNavDirection, TableNavResult, TableOp};
pub use utf16::{
  common_prefix_utf16, common_suffix_utf16, diff_edit_range, inserted_text_utf16,
  snap_utf16_boundary, utf16_len, utf16_next_boundary, utf16_prev_boundary, utf16_replace_range,
  utf16_slice, utf16_split_at, utf16_to_byte,
};
