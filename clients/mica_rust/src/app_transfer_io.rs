//! Application-level import/export & cross-workspace transfer orchestration.
//!
//! Extracted from `lib.rs` so file-dialog export/import wiring and recursive
//! cross-workspace subtree transfer stay cohesive and keep `lib.rs` well below
//! the 4,000-line soft size gate.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use mica_client::PageTreeModel;
use nsh_core::prelude::*;
use uuid::Uuid;

use crate::{
  WorkspaceEntry, cloud_sync, collect_all_views,
  editor_bridge::MicaDocumentBridge,
  storage::{DEFAULT_ORIGIN, VaultStore},
  transfer::{self, ArchiveExportFormat, ImportedPageSpec},
  transfer_dialog::{
    DestinationPick, TransferDestination, TransferDestinationNode, TransferDialogState,
    TransferPreviewReport,
  },
};

pub(crate) struct AppTransferContext {
  pub(crate) app_owner: Owner,
  pub(crate) vault: Rc<VaultStore>,
  pub(crate) bridge: Rc<MicaDocumentBridge>,
  pub(crate) tree: Rc<RefCell<PageTreeModel>>,
  pub(crate) docs_store: Rc<RefCell<HashMap<Uuid, String>>>,
  pub(crate) active_doc: Signal<Option<Uuid>>,
  pub(crate) active_origin: Signal<String>,
  pub(crate) active_title: Signal<String>,
  pub(crate) active_workspace_id: Signal<Uuid>,
  pub(crate) workspace_name: Signal<String>,
  pub(crate) workspaces: Signal<Vec<WorkspaceEntry>>,
  pub(crate) export_preview: Signal<String>,
  pub(crate) create_page_with_content: Rc<dyn Fn(String, String)>,
  pub(crate) import_page_specs: Rc<dyn Fn(Option<Uuid>, Vec<ImportedPageSpec>) -> usize>,
}

pub(crate) struct AppTransferHandlers {
  pub(crate) on_export_page_markdown: Rc<dyn Fn(Uuid)>,
  pub(crate) on_export_page_html: Rc<dyn Fn(Uuid)>,
  pub(crate) on_export_markdown: Rc<dyn Fn()>,
  pub(crate) on_export_html: Rc<dyn Fn()>,
  pub(crate) on_export_bundle: Rc<dyn Fn()>,
  pub(crate) export_workspace_zip: Rc<dyn Fn(Option<Uuid>, ArchiveExportFormat)>,
  pub(crate) export_folder_zip: Rc<dyn Fn(Uuid, ArchiveExportFormat)>,
  pub(crate) on_import_markdown: Rc<dyn Fn(String)>,
  pub(crate) on_import_zip_picker: Rc<dyn Fn(Option<Uuid>, bool)>,
  pub(crate) on_import_files_picker: Rc<dyn Fn(Option<Uuid>)>,
  pub(crate) on_import_folder_picker: Rc<dyn Fn(Option<Uuid>)>,
}

/// Build all native OS file-dialog export/import callbacks.
///
/// Every async dialog task is spawned under `app_owner` so closing a context
/// menu, workspace switcher, or app menu never cancels the native OS dialog.
pub(crate) fn build_transfer_handlers(ctx: AppTransferContext) -> AppTransferHandlers {
  let AppTransferContext {
    app_owner,
    vault,
    bridge,
    tree,
    docs_store,
    active_doc,
    active_origin,
    active_title,
    active_workspace_id,
    workspace_name,
    workspaces,
    export_preview,
    create_page_with_content,
    import_page_specs,
  } = ctx;

  let spawn_save_dialog = move |suggested: String,
                                filter_name: &'static str,
                                exts: &'static [&'static str],
                                bytes: Vec<u8>,
                                label: &'static str,
                                br_c: Rc<MicaDocumentBridge>| {
    app_owner.run(move || {
      spawn(async move {
        match transfer::save_bytes_with_dialog(&suggested, filter_name, exts, &bytes).await {
          Ok(Some(path)) => {
            export_preview.set(format!(
              "已导出 {label}：{}（{} 字节）",
              path.display(),
              bytes.len()
            ));
            br_c.set_status_flash(format!("已导出 {label}"));
          }
          Ok(None) => {}
          Err(err) => {
            export_preview.set(format!("导出 {label} 失败：{err}"));
            br_c.set_status_flash(format!("导出失败：{err}"));
          }
        }
      });
    });
  };

  let on_export_page_markdown: Rc<dyn Fn(Uuid)> = {
    let br = bridge.clone();
    let docs = docs_store.clone();
    let tree_ref = tree.clone();
    let v = vault.clone();
    Rc::new(move |doc_id: Uuid| {
      br.flush_pending_persist();
      let is_active = active_doc.get_untracked() == Some(doc_id);
      let orig = active_origin.get_untracked();
      let md = if is_active {
        br.export_markdown()
      } else {
        docs
          .borrow()
          .get(&doc_id)
          .cloned()
          .or_else(|| v.load_markdown(&orig, doc_id))
          .unwrap_or_default()
      };
      let title = if is_active {
        active_title.get_untracked()
      } else {
        tree_ref
          .borrow()
          .all_live_rows()
          .into_iter()
          .find(|r| !r.is_folder && (r.object_id == doc_id || r.view_id == doc_id))
          .map(|r| r.name)
          .unwrap_or_else(|| "untitled".to_string())
      };
      let suggested = format!("{}.md", transfer::sanitize_filename(&title, "untitled"));
      spawn_save_dialog(
        suggested,
        "Markdown",
        &["md"],
        md.into_bytes(),
        "Markdown",
        br.clone(),
      );
    })
  };

  let on_export_page_html: Rc<dyn Fn(Uuid)> = {
    let br = bridge.clone();
    let docs = docs_store.clone();
    let tree_ref = tree.clone();
    let v = vault.clone();
    Rc::new(move |doc_id: Uuid| {
      br.flush_pending_persist();
      let is_active = active_doc.get_untracked() == Some(doc_id);
      let orig = active_origin.get_untracked();
      let title = if is_active {
        active_title.get_untracked()
      } else {
        tree_ref
          .borrow()
          .all_live_rows()
          .into_iter()
          .find(|r| !r.is_folder && (r.object_id == doc_id || r.view_id == doc_id))
          .map(|r| r.name)
          .unwrap_or_else(|| "untitled".to_string())
      };
      let html_body = if is_active {
        br.export_html()
      } else {
        let md = docs
          .borrow()
          .get(&doc_id)
          .cloned()
          .or_else(|| v.load_markdown(&orig, doc_id))
          .unwrap_or_default();
        let blocks = mica_md::parse_body(&md, mica_md::ParseMode::Document);
        mica_md::render_html_body(&blocks, &mica_md::HtmlOptions::default())
      };
      let full_html = transfer::wrap_standalone_html(&title, &html_body);
      let suggested = format!("{}.html", transfer::sanitize_filename(&title, "untitled"));
      spawn_save_dialog(
        suggested,
        "HTML Document",
        &["html"],
        full_html.into_bytes(),
        "HTML",
        br.clone(),
      );
    })
  };

  let on_export_markdown: Rc<dyn Fn()> = {
    let exp = on_export_page_markdown.clone();
    let br = bridge.clone();
    Rc::new(move || {
      if let Some(doc_id) = active_doc.get_untracked() {
        exp(doc_id);
      } else {
        let suggested = format!(
          "{}.md",
          transfer::sanitize_filename(&active_title.get_untracked(), "untitled")
        );
        spawn_save_dialog(
          suggested,
          "Markdown",
          &["md"],
          br.export_markdown().into_bytes(),
          "Markdown",
          br.clone(),
        );
      }
    })
  };

  let on_export_html: Rc<dyn Fn()> = {
    let exp = on_export_page_html.clone();
    let br = bridge.clone();
    Rc::new(move || {
      if let Some(doc_id) = active_doc.get_untracked() {
        exp(doc_id);
      } else {
        let title = active_title.get_untracked();
        let full = transfer::wrap_standalone_html(&title, &br.export_html());
        let suggested = format!("{}.html", transfer::sanitize_filename(&title, "untitled"));
        spawn_save_dialog(
          suggested,
          "HTML Document",
          &["html"],
          full.into_bytes(),
          "HTML",
          br.clone(),
        );
      }
    })
  };

  let on_export_bundle: Rc<dyn Fn()> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let br = bridge.clone();
    Rc::new(move || {
      br.flush_pending_persist();
      if let Some(aid) = active_doc.get_untracked() {
        docs.borrow_mut().insert(aid, br.export_markdown());
      }
      let ws_name = workspace_name.get_untracked();
      let (live, _) = collect_all_views(&tree_ref.borrow());
      let docs_snap = docs.borrow().clone();
      let pages: Vec<(String, String)> = live
        .into_iter()
        .filter(cloud_sync::is_page_view)
        .map(|v| (v.name, docs_snap.get(&v.object_id).cloned().unwrap_or_default()))
        .collect();
      match transfer::serialize_workspace_bundle(&ws_name, &pages) {
        Ok(json) => {
          let suggested = format!(
            "{}.mica.json",
            transfer::sanitize_filename(&ws_name, "mica-workspace")
          );
          spawn_save_dialog(
            suggested,
            "Mica Backup JSON",
            &["json"],
            json.into_bytes(),
            "工作区备份",
            br.clone(),
          );
        }
        Err(err) => {
          export_preview.set(format!("导出备份失败：{err}"));
          br.set_status_flash(format!("导出失败：{err}"));
        }
      }
    })
  };

  let export_workspace_zip: Rc<dyn Fn(Option<Uuid>, ArchiveExportFormat)> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let v = vault.clone();
    let br = bridge.clone();
    Rc::new(move |target_ws: Option<Uuid>, fmt: ArchiveExportFormat| {
      br.flush_pending_persist();
      if let Some(aid) = active_doc.get_untracked() {
        docs.borrow_mut().insert(aid, br.export_markdown());
      }
      let cur_ws = active_workspace_id.get_untracked();
      let want_ws = target_ws.unwrap_or(cur_ws);
      let (ws_name, live, docs_map) = if want_ws == cur_ws {
        let (live, _) = collect_all_views(&tree_ref.borrow());
        (workspace_name.get_untracked(), live, docs.borrow().clone())
      } else {
        let ws_info = workspaces.get_untracked().into_iter().find(|w| w.id == want_ws);
        let (name, orig) = ws_info
          .map(|w| (w.name, w.origin))
          .unwrap_or_else(|| ("workspace".to_string(), DEFAULT_ORIGIN.to_string()));
        let live: Vec<mica_api_types::View> = v
          .list_views(&orig)
          .into_iter()
          .filter(|row| !row.is_deleted && row.workspace_id == want_ws)
          .collect();
        let mut map = HashMap::new();
        for row in &live {
          if cloud_sync::is_page_view(row)
            && let Some(md) = v.load_markdown(&orig, row.object_id)
          {
            map.insert(row.object_id, md);
          }
        }
        (name, live, map)
      };
      match transfer::build_workspace_zip_bytes(&ws_name, &live, &docs_map, fmt) {
        Ok(bytes) => {
          let ext = if fmt == ArchiveExportFormat::BackupMicaZip {
            "mica.zip"
          } else {
            "zip"
          };
          let suggested = format!("{}.{ext}", transfer::sanitize_filename(&ws_name, "workspace"));
          spawn_save_dialog(suggested, "ZIP Archive", &["zip"], bytes, "ZIP", br.clone());
        }
        Err(e) => {
          export_preview.set(format!("打包 ZIP 失败：{e}"));
          br.set_status_flash(format!("打包失败：{e}"));
        }
      }
    })
  };

  let export_folder_zip: Rc<dyn Fn(Uuid, ArchiveExportFormat)> = {
    let tree_ref = tree.clone();
    let docs = docs_store.clone();
    let br = bridge.clone();
    Rc::new(move |root_id: Uuid, fmt: ArchiveExportFormat| {
      br.flush_pending_persist();
      if let Some(aid) = active_doc.get_untracked() {
        docs.borrow_mut().insert(aid, br.export_markdown());
      }
      let (live, _) = collect_all_views(&tree_ref.borrow());
      let folder_name = live
        .iter()
        .find(|v| v.id == root_id || v.object_id == root_id)
        .map(|v| v.name.clone())
        .unwrap_or_else(|| "export".to_string());
      let docs_map = docs.borrow().clone();
      match transfer::build_subtree_zip_bytes(root_id, &live, &docs_map, fmt) {
        Ok(bytes) => {
          let ext = if fmt == ArchiveExportFormat::BackupMicaZip {
            "mica.zip"
          } else {
            "zip"
          };
          let suggested = format!("{}.{ext}", transfer::sanitize_filename(&folder_name, "export"));
          spawn_save_dialog(suggested, "ZIP Archive", &["zip"], bytes, "ZIP", br.clone());
        }
        Err(e) => {
          export_preview.set(format!("打包失败：{e}"));
          br.set_status_flash(format!("打包失败：{e}"));
        }
      }
    })
  };

  let on_import_markdown: Rc<dyn Fn(String)> = {
    let create_page = create_page_with_content.clone();
    Rc::new(move |path: String| match transfer::import_markdown_from_path(&path) {
      Ok((title, md)) => {
        export_preview.set(format!("已导入页面「{}」（{} 字节）", title, md.len()));
        create_page(title, md);
      }
      Err(err) => export_preview.set(format!("导入失败：{err}")),
    })
  };

  let on_import_zip_picker: Rc<dyn Fn(Option<Uuid>, bool)> = {
    let imp = import_page_specs.clone();
    let br = bridge.clone();
    Rc::new(move |parent: Option<Uuid>, notion: bool| {
      let imp_c = imp.clone();
      let br_c = br.clone();
      app_owner.run(move || {
        spawn(async move {
          match transfer::pick_import_file_bytes("ZIP Archive", &["zip"], false).await {
            Ok(files) if !files.is_empty() => {
              match transfer::parse_zip_archive_to_pages(&files[0].bytes, notion) {
                Ok(specs) => {
                  let n = imp_c(parent, specs);
                  export_preview.set(format!("已从 ZIP 导入 {n} 篇页面"));
                  br_c.set_status_flash(format!("已导入 {n} 篇页面"));
                }
                Err(e) => {
                  export_preview.set(format!("解析 ZIP 失败：{e}"));
                  br_c.set_status_flash(format!("导入失败：{e}"));
                }
              }
            }
            Ok(_) => {}
            Err(e) => {
              export_preview.set(format!("选择文件失败：{e}"));
              br_c.set_status_flash(format!("选择文件失败：{e}"));
            }
          }
        });
      });
    })
  };

  let on_import_files_picker: Rc<dyn Fn(Option<Uuid>)> = {
    let imp = import_page_specs.clone();
    let br = bridge.clone();
    Rc::new(move |parent: Option<Uuid>| {
      let imp_c = imp.clone();
      let br_c = br.clone();
      app_owner.run(move || {
        spawn(async move {
          match transfer::pick_import_file_bytes(
            "Markdown / ZIP / SVG",
            &["md", "markdown", "txt", "zip", "svg"],
            true,
          )
          .await
          {
            Ok(files) if !files.is_empty() => {
              let mut all_specs = Vec::new();
              for f in files {
                if let Ok(specs) = transfer::parse_picked_file_bytes(&f.name, &f.bytes, false) {
                  all_specs.extend(specs);
                }
              }
              let n = imp_c(parent, all_specs);
              export_preview.set(format!("已导入 {n} 篇页面"));
              br_c.set_status_flash(format!("已导入 {n} 篇页面"));
            }
            Ok(_) => {}
            Err(e) => {
              export_preview.set(format!("选择文件失败：{e}"));
              br_c.set_status_flash(format!("选择文件失败：{e}"));
            }
          }
        });
      });
    })
  };

  let on_import_folder_picker: Rc<dyn Fn(Option<Uuid>)> = {
    let imp = import_page_specs.clone();
    let br = bridge.clone();
    Rc::new(move |parent: Option<Uuid>| {
      let imp_c = imp.clone();
      let br_c = br.clone();
      app_owner.run(move || {
        spawn(async move {
          match transfer::pick_import_folder().await {
            Ok(Some(dir)) => match transfer::import_folder_from_disk(&dir, true) {
              Ok(specs) => {
                let n = imp_c(parent, specs);
                export_preview.set(format!("已从文件夹导入 {n} 篇页面"));
                br_c.set_status_flash(format!("已导入 {n} 篇页面"));
              }
              Err(e) => {
                export_preview.set(format!("导入文件夹失败：{e}"));
                br_c.set_status_flash(format!("导入失败：{e}"));
              }
            },
            Ok(None) => {}
            Err(e) => {
              export_preview.set(format!("选择文件夹失败：{e}"));
              br_c.set_status_flash(format!("选择文件夹失败：{e}"));
            }
          }
        });
      });
    })
  };

  AppTransferHandlers {
    on_export_page_markdown,
    on_export_page_html,
    on_export_markdown,
    on_export_html,
    on_export_bundle,
    export_workspace_zip,
    export_folder_zip,
    on_import_markdown,
    on_import_zip_picker,
    on_import_files_picker,
    on_import_folder_picker,
  }
}

pub(crate) struct CrossWorkspaceTransferCtx {
  pub(crate) vault: Rc<VaultStore>,
  pub(crate) reload_tree: Rc<dyn Fn()>,
  pub(crate) active_workspace_id: Signal<Uuid>,
  pub(crate) active_origin: Signal<String>,
  pub(crate) workspaces: Signal<Vec<WorkspaceEntry>>,
  pub(crate) transfer_source_ids: Signal<Vec<Uuid>>,
  pub(crate) transfer_copy_mode: Signal<bool>,
  pub(crate) transfer_modal_open: Signal<bool>,
}

/// Build the `TransferDialogState` for the 440px cross-workspace copy/move modal.
pub(crate) fn build_transfer_dialog_state(ctx: CrossWorkspaceTransferCtx) -> TransferDialogState {
  let CrossWorkspaceTransferCtx {
    vault,
    reload_tree,
    active_workspace_id,
    active_origin,
    workspaces,
    transfer_source_ids,
    transfer_copy_mode,
    transfer_modal_open,
  } = ctx;

  let cur_ws = active_workspace_id.get_untracked();
  let dests: Vec<TransferDestination> = workspaces
    .get_untracked()
    .into_iter()
    .filter(|w| w.id != cur_ws)
    .map(|w| TransferDestination {
      id: w.id,
      name: w.name,
    })
    .collect();
  let vt1 = vault.clone();
  let vt2 = vault;
  let src_ids = transfer_source_ids.get_untracked();
  let copy_mode = transfer_copy_mode.get_untracked();

  TransferDialogState {
    copy: copy_mode,
    item_count: src_ids.len().max(1),
    destinations: dests,
    on_load_tree: Rc::new(move |dest_ws: Uuid| {
      let orig = workspaces
        .get_untracked()
        .into_iter()
        .find(|w| w.id == dest_ws)
        .map(|w| w.origin)
        .unwrap_or_else(|| DEFAULT_ORIGIN.to_string());
      vt1
        .list_views(&orig)
        .into_iter()
        .filter(|v| !v.is_deleted)
        .map(|v| TransferDestinationNode::from(&v))
        .collect()
    }),
    on_preview: Rc::new(move |_dest_ws: Uuid, _pick: DestinationPick| {
      Ok(TransferPreviewReport {
        documents: src_ids.len().max(1),
        folders: 0,
        images: 0,
        dangling_links: 0,
      })
    }),
    on_confirm: Rc::new(move |dest_ws: Uuid, pick: DestinationPick| {
      let src_orig = active_origin.get_untracked();
      let dest_orig = workspaces
        .get_untracked()
        .into_iter()
        .find(|w| w.id == dest_ws)
        .map(|w| w.origin)
        .ok_or_else(|| "未找到目标工作区".to_string())?;
      let src_views = vt2.list_views(&src_orig);
      for sid in transfer_source_ids.get_untracked() {
        if let Some(vrow) = src_views.iter().find(|r| r.id == sid || r.object_id == sid) {
          clone_subtree_to_dest(
            &vt2,
            &src_orig,
            &dest_orig,
            dest_ws,
            &src_views,
            vrow,
            pick.parent_view_id,
          );
          if !copy_mode {
            let _ = vt2.trash_view_subtree(&src_orig, vrow.id);
          }
        }
      }
      reload_tree();
      transfer_modal_open.set(false);
      Ok(())
    }),
    on_close: Rc::new(move || transfer_modal_open.set(false)),
  }
}

fn clone_subtree_to_dest(
  vt: &VaultStore,
  src_orig: &str,
  dest_orig: &str,
  dest_ws: Uuid,
  src_views: &[mica_api_types::View],
  vrow: &mica_api_types::View,
  parent_id: Option<Uuid>,
) {
  let mut cloned = vrow.clone();
  cloned.id = Uuid::new_v4();
  cloned.workspace_id = dest_ws;
  cloned.parent_view_id = parent_id;
  if cloud_sync::is_page_view(&cloned) {
    let new_obj = Uuid::new_v4();
    let md = vt.load_markdown(src_orig, vrow.object_id).unwrap_or_default();
    cloned.object_id = new_obj;
    let _ = vt.save_markdown(dest_orig, new_obj, &md);
    vt.record_doc_created(
      dest_orig,
      dest_ws,
      new_obj,
      mica_core::ActivitySource::Transfer,
      &md,
    );
    let _ = vt.save_view(dest_orig, &cloned);
  } else {
    cloned.object_id = cloned.id;
    let _ = vt.save_view(dest_orig, &cloned);
    for child in src_views
      .iter()
      .filter(|c| !c.is_deleted && c.parent_view_id == Some(vrow.id))
    {
      clone_subtree_to_dest(
        vt,
        src_orig,
        dest_orig,
        dest_ws,
        src_views,
        child,
        Some(cloned.id),
      );
    }
  }
}
