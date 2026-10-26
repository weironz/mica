//! Mica's import/export interchange engine — pure, I/O-free, dependency-free
//! (serde_json only, for the export manifest). Bytes and structures in,
//! plans and archive entries out; the api-server executes the side effects
//! (database writes, object storage). See docs/features/export-import.md.

pub mod export_tree;
pub mod import;
pub mod normalize;
pub mod notion;
pub mod order;
pub mod zip;

pub use export_tree::{
  ImageAsset, TreeExport, TreeNode, build_incremental_markdown_tree_zip, build_markdown_tree_zip,
  for_each_inline_mark_mut, markdown_tree_manifest, safe_segment, unique_asset_name,
};
pub use import::{
  ImportMode, ImportPlan, PagePlan, percent_decode, plan_import, plan_vault_import, resolve_ref,
};
pub use normalize::{expand_nested_zips, normalize_entries};
pub use zip::{ZipEntry, ZipFileEntry, ZipStreamWriter, build_zip, read_zip};
