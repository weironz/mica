//! Extracted root application modal/dialog state builders and pure view/tree
//! helpers (`ShareDialogState`, AI dialogs, workspace switcher callbacks, version
//! formatting, backlinks computation, and startup helpers) keeping `lib.rs` concise.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use chrono::{DateTime, Local, Utc};
use mica_api_types::View;
use mica_client::PageTreeModel;
#[cfg(not(target_arch = "wasm32"))]
use nsh_core::desktop::{TitleBarOptions, TitleBarStyle, WindowAttributes};
#[cfg(not(target_arch = "wasm32"))]
use nsh_core::element::desktop::current_window;
#[cfg(not(target_arch = "wasm32"))]
use nsh_core::geometry::px;
use nsh_core::prelude::*;
use uuid::Uuid;

#[cfg(not(target_arch = "wasm32"))]
use crate::APP_VERSION;
use crate::{
  WorkspaceEntry,
  ai_dialog::{self, AiTarget, GlobalAiState, InlineAiPhase, InlineAiState},
  cloud_sync::{self, CloudSessionHandles},
  editor_bridge::MicaDocumentBridge,
  net::CloudConnector,
  right_sidebar::{BacklinkEntry, SnapshotEntry},
  share_dialog::ShareDialogState,
  sidebar::WorkspaceSwitcherCallbacks,
  storage::{DEFAULT_ORIGIN, VaultStore},
  transfer::ArchiveExportFormat,
  version_history::{VersionGroup, VersionHistoryItem, format_version_stamp_now},
};

pub(crate) fn configure_hidden_title_bar() {
  #[cfg(not(target_arch = "wasm32"))]
  {
    let window = current_window();
    let mut tb = TitleBarOptions::default();
    tb.style = TitleBarStyle::Hidden;
    tb.height = Some(px(34.0));
    let mut attrs = WindowAttributes::default();
    attrs.title_bar = Some(tb);
    window.set(attrs);
  }
}

/// Writes a non-empty startup receipt once the root window produces its first
/// non-zero layout frame when `MICA_STARTUP_PROBE_FILE` is set by installer /
/// release canary smoke harnesses.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn record_startup_probe_once() {
  static PROBE_WRITTEN: std::sync::Once = std::sync::Once::new();
  PROBE_WRITTEN.call_once(|| {
    if let Some(path) = std::env::var_os("MICA_STARTUP_PROBE_FILE")
      && !path.is_empty()
    {
      let path = std::path::PathBuf::from(path);
      if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
      }
      let _ = std::fs::write(&path, format!("Mica {APP_VERSION}\n"));
    }
  });
}

pub(crate) fn format_version_entries(
  vault: &VaultStore,
  origin: &str,
  doc_id: Uuid,
) -> (Vec<SnapshotEntry>, Vec<String>, Vec<VersionHistoryItem>) {
  let raw = vault.list_versions(origin, doc_id);
  let mut entries = Vec::with_capacity(raw.len());
  let mut ids = Vec::with_capacity(raw.len());
  let mut history_items = Vec::with_capacity(raw.len());
  for (idx, (ver_id, label, ts_ms)) in raw.into_iter().enumerate() {
    let dt_iso = DateTime::from_timestamp_millis(ts_ms)
      .map(|d| d.to_rfc3339())
      .unwrap_or_default();
    let (group, stamp) = if ts_ms > 0 {
      format_version_stamp_now(&dt_iso)
    } else {
      (VersionGroup::Today, "刚刚".to_string())
    };
    let created_short = if ts_ms > 0 {
      DateTime::from_timestamp_millis(ts_ms)
        .map(|dt| dt.with_timezone(&Local).format("%m/%d %H:%M").to_string())
        .unwrap_or_else(|| "刚刚".to_string())
    } else {
      "刚刚".to_string()
    };
    let is_named = !label.is_empty() && label != "自动快照";
    entries.push(SnapshotEntry {
      id: idx as i64,
      label: label.clone(),
      created_at: created_short,
    });
    history_items.push(VersionHistoryItem {
      id: ver_id.clone(),
      label: if is_named { label } else { stamp.clone() },
      is_named,
      created_at: stamp,
      author: None,
      is_today: matches!(group, VersionGroup::Today),
    });
    ids.push(ver_id);
  }
  (entries, ids, history_items)
}

pub(crate) fn collect_all_views(tree: &PageTreeModel) -> (Vec<View>, Vec<View>) {
  let mut live = Vec::new();
  let mut trashed = Vec::new();
  for row in tree.all_live_rows() {
    if let Some(v) = tree.get(row.view_id).cloned() {
      live.push(v);
    }
  }
  for v in tree.trashed_views() {
    trashed.push(v.clone());
  }
  (live, trashed)
}

pub(crate) fn compute_backlinks_for(
  tree: &PageTreeModel,
  docs: &HashMap<Uuid, String>,
  target_doc: Option<Uuid>,
  target_title: &str,
) -> Vec<BacklinkEntry> {
  let Some(target_id) = target_doc else {
    return Vec::new();
  };
  let id_needle = format!("mica://page/{target_id}");
  let wiki_needle = if target_title.trim().is_empty() {
    None
  } else {
    Some(format!("[[{}]]", target_title.trim()))
  };
  let mut out = Vec::new();
  for row in tree.all_live_rows() {
    if row.is_folder || row.object_id == target_id {
      continue;
    }
    if let Some(body) = docs.get(&row.object_id) {
      let matches_id = body.contains(&id_needle);
      let matches_wiki = wiki_needle
        .as_ref()
        .is_some_and(|w| body.contains(w.as_str()));
      if matches_id || matches_wiki {
        out.push(BacklinkEntry {
          doc_id: row.object_id,
          title: row.name,
        });
      }
    }
  }
  out
}

pub(crate) fn resolve_parent_folder_name(tree: &PageTreeModel, doc_id: Uuid) -> Option<String> {
  let rows = tree.all_live_rows();
  let page = rows.iter().find(|r| !r.is_folder && r.object_id == doc_id)?;
  let mut ancestors = Vec::new();
  let mut cursor = page.parent_view_id;
  let mut seen = std::collections::HashSet::new();
  while let Some(pid) = cursor {
    if !seen.insert(pid) {
      break;
    }
    let Some(folder) = tree.get(pid) else {
      break;
    };
    if !folder.name.trim().is_empty() {
      ancestors.push(folder.name.trim().to_string());
    }
    cursor = folder.parent_view_id;
  }
  if ancestors.is_empty() {
    None
  } else {
    ancestors.reverse();
    Some(ancestors.join(" > "))
  }
}

pub(crate) fn make_view(
  ws_id: Uuid,
  object_id: Uuid,
  name: &str,
  object_type: &str,
  parent_view_id: Option<Uuid>,
  position: &str,
) -> View {
  let now = Utc::now();
  View {
    id: if object_type == "folder" {
      object_id
    } else {
      Uuid::new_v4()
    },
    workspace_id: ws_id,
    object_id,
    name: name.to_string(),
    object_type: object_type.to_string(),
    icon: None,
    parent_view_id,
    position: position.to_string(),
    is_deleted: false,
    created_by: Uuid::nil(),
    created_at: now,
    updated_at: now,
    state_bytes: None,
  }
}

pub(crate) struct ShareModalCtx {
  pub(crate) app_owner: Owner,
  pub(crate) allow_local: bool,
  pub(crate) vault: Rc<VaultStore>,
  pub(crate) cloud_connector: Rc<RefCell<CloudConnector>>,
  pub(crate) active_origin: Signal<String>,
  pub(crate) active_workspace_id: Signal<Uuid>,
  pub(crate) active_doc: Signal<Option<Uuid>>,
  pub(crate) is_cloud_signed_in: Signal<bool>,
  pub(crate) server_url: Signal<String>,
  pub(crate) account_email: Signal<String>,
  pub(crate) open: Signal<bool>,
  pub(crate) loading: Signal<bool>,
  pub(crate) busy: Signal<bool>,
  pub(crate) shared: Signal<bool>,
  pub(crate) token: Signal<Option<String>>,
  pub(crate) error: Signal<Option<String>>,
}

pub(crate) fn build_share_dialog_state(ctx: ShareModalCtx) -> ShareDialogState {
  let ShareModalCtx {
    app_owner,
    allow_local,
    vault,
    cloud_connector,
    active_origin,
    active_workspace_id,
    active_doc,
    is_cloud_signed_in,
    server_url,
    account_email,
    open,
    loading,
    busy,
    shared,
    token,
    error,
  } = ctx;
  ShareDialogState {
    open,
    loading,
    busy,
    shared,
    token,
    origin: server_url,
    error,
    on_toggle_public: {
      let toggle_gen = signal(0u64);
      Rc::new(move |enable: bool| {
        let orig = active_origin.get_untracked();
        if !cloud_sync::is_cloud_origin(&orig) || !is_cloud_signed_in.get_untracked() {
          error.set(Some("公开分享仅支持云端工作区，请先登录云端账号。".to_string()));
          return;
        }
        let Some(doc_id) = active_doc.get_untracked() else {
          return;
        };
        let ws = active_workspace_id.get_untracked();
        let srv_req = server_url.get_untracked();
        let email_req = account_email.get_untracked();
        let cc_a = cloud_connector.clone();
        let v_a = vault.clone();
        let epoch = toggle_gen.get_untracked().wrapping_add(1);
        toggle_gen.set(epoch);
        busy.set(true);
        error.set(None);
        // Spawn under `app_owner` (not the transient `dynamic(...)` button owner),
        // because setting `busy = true` immediately rebuilds the dialog body
        // and would otherwise cancel this task before the HTTP request completes.
        app_owner.run(move || {
          spawn(async move {
            let mut conn = cc_a.borrow().clone();
            let res = cloud_sync::with_auto_refresh(&mut conn, move |c| async move {
              if enable {
                c.create_share(ws, doc_id).await
              } else {
                c.delete_share(ws, doc_id).await.map(|_| crate::net::ShareState {
                  shared: false,
                  token: None,
                  url: None,
                })
              }
            })
            .await
            .map(|(st, rot)| (conn, st, rot));
            if toggle_gen.get_untracked() != epoch {
              return;
            }
            busy.set(false);
            if active_origin.get_untracked() != orig || active_doc.get_untracked() != Some(doc_id) {
              return;
            }
            match res {
              Ok((conn, st, rot)) => {
                if rot.is_some() {
                  *cc_a.borrow_mut() = conn;
                  if allow_local {
                    cloud_sync::persist_refreshed_tokens(&v_a, &srv_req, &email_req, rot);
                  }
                }
                shared.set(st.shared);
                token.set(st.token);
                error.set(None);
              }
              Err(e) => error.set(Some(e)),
            }
          });
        });
      })
    },
    on_close: Rc::new(move || open.set(false)),
  }
}

pub(crate) struct AiDialogsCtx {
  pub(crate) app_owner: Owner,
  pub(crate) cloud_connector: Rc<RefCell<CloudConnector>>,
  pub(crate) bridge: Rc<MicaDocumentBridge>,
  pub(crate) create_page_with_content: Rc<dyn Fn(String, String)>,
  pub(crate) global_ai_open: Signal<bool>,
  pub(crate) global_ai_prompt: Signal<String>,
  pub(crate) global_ai_target: Signal<AiTarget>,
  pub(crate) global_ai_output: Signal<String>,
  pub(crate) global_ai_streaming: Signal<bool>,
  pub(crate) global_ai_done: Signal<bool>,
  pub(crate) global_ai_error: Signal<Option<String>>,
  pub(crate) inline_ai_open: Signal<bool>,
  pub(crate) inline_ai_prompt: Signal<String>,
  pub(crate) inline_ai_context: Signal<String>,
  pub(crate) inline_ai_allow_replace: Signal<bool>,
  pub(crate) inline_ai_output: Signal<String>,
  pub(crate) inline_ai_phase: Signal<InlineAiPhase>,
  pub(crate) inline_ai_error: Signal<Option<String>>,
}

pub(crate) fn build_ai_dialog_states(ctx: AiDialogsCtx) -> (GlobalAiState, InlineAiState) {
  let AiDialogsCtx {
    app_owner,
    cloud_connector,
    bridge,
    create_page_with_content,
    global_ai_open,
    global_ai_prompt,
    global_ai_target,
    global_ai_output,
    global_ai_streaming,
    global_ai_done,
    global_ai_error,
    inline_ai_open,
    inline_ai_prompt,
    inline_ai_context,
    inline_ai_allow_replace,
    inline_ai_output,
    inline_ai_phase,
    inline_ai_error,
  } = ctx;

  let global_state = GlobalAiState {
    open: global_ai_open,
    can_edit: signal(true),
    has_workspace: signal(true),
    has_current_page: signal(true),
    edit_mode: signal(false),
    prompt: global_ai_prompt,
    target: global_ai_target,
    output: global_ai_output,
    edit_diffs: signal(Vec::new()),
    streaming: global_ai_streaming,
    done: global_ai_done,
    applying: signal(false),
    error: global_ai_error,
    on_generate: {
      let cc = cloud_connector.clone();
      let ai_gen = signal(0u64);
      Rc::new(move |prompt: String, _target: AiTarget| {
        let epoch = ai_gen.get_untracked().wrapping_add(1);
        ai_gen.set(epoch);
        global_ai_streaming.set(true);
        global_ai_error.set(None);
        let conn = cc.borrow().clone();
        app_owner.run(move || {
          spawn(async move {
            let res = conn
              .stream_ai_prompt("你是 Mica 知识库写作助手，请直接输出干净的 Markdown。", &prompt)
              .await;
            if ai_gen.get_untracked() != epoch {
              return;
            }
            match res {
              Ok(out) => {
                global_ai_output.set(out);
                global_ai_done.set(true);
              }
              Err(e) => global_ai_error.set(Some(e)),
            }
            global_ai_streaming.set(false);
          });
        });
      })
    },
    on_apply: {
      let br = bridge.clone();
      Rc::new(move |md: String, target: AiTarget| {
        match target {
          AiTarget::NewPage | AiTarget::NewWorkspace => {
            create_page_with_content("AI 生成页面".to_string(), md);
          }
          AiTarget::CurrentPage => {
            br.append_markdown_preserving_crdt(&md);
          }
        }
        global_ai_open.set(false);
      })
    },
    on_close: Rc::new(move || global_ai_open.set(false)),
  };

  let inline_state = InlineAiState {
    open: inline_ai_open,
    prompt: inline_ai_prompt,
    context_text: inline_ai_context,
    allow_replace: inline_ai_allow_replace,
    output: inline_ai_output,
    phase: inline_ai_phase,
    error: inline_ai_error,
    on_generate: {
      let cc = cloud_connector;
      let ai_gen = signal(0u64);
      Rc::new(move |prompt: String, ctx_text: String| {
        let epoch = ai_gen.get_untracked().wrapping_add(1);
        ai_gen.set(epoch);
        inline_ai_phase.set(InlineAiPhase::Running);
        inline_ai_error.set(None);
        let full = if ctx_text.is_empty() {
          prompt
        } else {
          format!("{prompt}\n\n原文：\n{ctx_text}")
        };
        let conn = cc.borrow().clone();
        app_owner.run(move || {
          spawn(async move {
            let res = conn.stream_ai_prompt(ai_dialog::K_AI_REWRITE_SYSTEM, &full).await;
            if ai_gen.get_untracked() != epoch {
              return;
            }
            match res {
              Ok(out) => {
                inline_ai_output.set(out);
                inline_ai_phase.set(InlineAiPhase::Succeeded);
              }
              Err(e) => {
                inline_ai_error.set(Some(e));
                inline_ai_phase.set(InlineAiPhase::Failed);
              }
            }
          });
        });
      })
    },
    on_insert_below: {
      let br = bridge.clone();
      Rc::new(move |md: String| {
        br.insert_custom_block("paragraph", md.trim(), None);
        inline_ai_open.set(false);
      })
    },
    on_replace_selection: {
      let br = bridge;
      Rc::new(move |md: String| {
        br.insert_custom_block("paragraph", md.trim(), None);
        inline_ai_open.set(false);
      })
    },
    on_close: Rc::new(move || inline_ai_open.set(false)),
  };

  (global_state, inline_state)
}

pub(crate) struct WsSwitcherCtx {
  pub(crate) vault: Rc<VaultStore>,
  pub(crate) cloud_session: Rc<CloudSessionHandles>,
  pub(crate) workspaces: Signal<Vec<WorkspaceEntry>>,
  pub(crate) active_origin: Signal<String>,
  pub(crate) active_workspace_id: Signal<Uuid>,
  pub(crate) workspace_name: Signal<String>,
  pub(crate) workspace_menu_open: Signal<bool>,
  pub(crate) is_cloud_signed_in: Signal<bool>,
  pub(crate) on_switch_workspace: Rc<dyn Fn(WorkspaceEntry)>,
  pub(crate) export_workspace_zip: Rc<dyn Fn(Option<Uuid>, ArchiveExportFormat)>,
  pub(crate) on_import_files_picker: Rc<dyn Fn(Option<Uuid>)>,
  pub(crate) on_import_folder_picker: Rc<dyn Fn(Option<Uuid>)>,
  pub(crate) on_import_zip_picker: Rc<dyn Fn(Option<Uuid>, bool)>,
}

pub(crate) fn build_ws_switcher_callbacks(ctx: WsSwitcherCtx) -> Rc<WorkspaceSwitcherCallbacks> {
  let WsSwitcherCtx {
    vault,
    cloud_session,
    workspaces,
    active_origin,
    active_workspace_id,
    workspace_name,
    workspace_menu_open,
    is_cloud_signed_in,
    on_switch_workspace,
    export_workspace_zip,
    on_import_files_picker,
    on_import_folder_picker,
    on_import_zip_picker,
  } = ctx;

  Rc::new(WorkspaceSwitcherCallbacks {
    on_select: {
      let sw = on_switch_workspace.clone();
      Rc::new(move |id: Uuid| {
        if let Some(entry) = workspaces.get_untracked().into_iter().find(|w| w.id == id) {
          sw(entry);
        }
      })
    },
    on_create: {
      let sw = on_switch_workspace.clone();
      let v = vault.clone();
      let cs = cloud_session.clone();
      Rc::new(move |name: String| {
        let cur_orig = active_origin.get_untracked();
        if cloud_sync::is_cloud_origin(&cur_orig) && is_cloud_signed_in.get_untracked() {
          workspace_menu_open.set(false);
          cs.create_cloud_workspace(cur_orig, name);
          return;
        }
        let id = Uuid::new_v4();
        let origin = DEFAULT_ORIGIN.to_string();
        let _ = v.save_workspace_name(&origin, id, &name);
        let entry = WorkspaceEntry { id, name, origin, page_count: None };
        let mut all = workspaces.get_untracked();
        all.push(entry.clone());
        workspaces.set(all);
        sw(entry);
      })
    },
    on_rename: {
      let v = vault.clone();
      let cs = cloud_session.clone();
      Rc::new(move |id: Uuid, next_name: String| {
        let mut all = workspaces.get_untracked();
        for w in &mut all {
          if w.id == id {
            let _ = v.save_workspace_name(&w.origin, id, &next_name);
            w.name = next_name.clone();
          }
        }
        workspaces.set(all);
        let name_req = next_name.clone();
        cs.spawn_cloud_op("云端重命名工作区", move |c| {
          let name_req = name_req.clone();
          async move { c.update_workspace(id, &name_req).await }
        });
        if active_workspace_id.get_untracked() == id {
          workspace_name.set(next_name);
        }
      })
    },
    on_export_backup: {
      let exp = export_workspace_zip.clone();
      Rc::new(move |ws_id: Uuid| exp(Some(ws_id), ArchiveExportFormat::BackupMicaZip))
    },
    on_export_markdown: {
      Rc::new(move |ws_id: Uuid| export_workspace_zip(Some(ws_id), ArchiveExportFormat::MarkdownZip))
    },
    on_import_files: Rc::new(move |_| on_import_files_picker(None)),
    on_import_folder: Rc::new(move |_| on_import_folder_picker(None)),
    on_delete: {
      let sw = on_switch_workspace;
      let v = vault;
      let cs = cloud_session;
      Rc::new(move |id: Uuid| {
        let mut all = workspaces.get_untracked();
        if all.len() <= 1 {
          return;
        }
        if let Some(target) = all.iter().find(|w| w.id == id) {
          let _ = v.delete_workspace(&target.origin, id);
        }
        cs.spawn_cloud_op("云端删除工作区", move |c| async move { c.delete_workspace(id).await });
        all.retain(|w| w.id != id);
        let next = all[0].clone();
        workspaces.set(all);
        if active_workspace_id.get_untracked() == id {
          sw(next);
        }
      })
    },
    on_import_workspace_zip: {
      let imp = on_import_zip_picker.clone();
      Rc::new(move || imp(None, false))
    },
    on_import_workspace_notion: Rc::new(move || on_import_zip_picker(None, true)),
    on_close: Rc::new(move || workspace_menu_open.set(false)),
  })
}

pub(crate) const STARTER_MARKDOWN: &str = "\
# 欢迎使用 Mica

这是一个 100% 全栈纯 Rust 知识库与无限白板工作区，支持本地离线 SQLite 存储与云端 CRDT 同步。

## 核心能力

- 原生富文本块编辑：跨块鼠标拖选、多块框选、回车分块、行首退格合并、`Ctrl+A` 全选与原位中文 IME
- 真实多行代码编辑器、表格、RaTeX 公式、Mermaid 流程图与内嵌无限白板
- 无边框融合标题栏、应用菜单、可拖拽调宽正文与侧栏、右键/多选拖拽目录树、关系图谱与版本历史 Diff

```rust
fn main() {
    println!(\"Hello from Mica Pure Rust!\");
}
```
";
