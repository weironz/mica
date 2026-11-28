//! Cohesive `Copy` signal bundles for root application shell state (`lib.rs`).

use nsh_core::prelude::*;
use uuid::Uuid;

use crate::{
  ai_dialog::{AiTarget, InlineAiPhase},
  app_chrome::EmbeddedBoardEdit,
  graph_dialog::PageGraphData,
};

/// Modal / popover open flags and draft signals owned at the app root.
#[derive(Clone, Copy)]
pub(crate) struct ModalSignals {
  pub app_menu_open: Signal<bool>,
  pub about_modal_open: Signal<bool>,
  pub world_switcher_open: Signal<bool>,
  pub world_can_close: Signal<bool>,
  pub search_modal_open: Signal<bool>,
  pub search_initial_query: Signal<Option<String>>,
  pub search_open_in_new_tab: Signal<bool>,
  pub settings_modal_open: Signal<bool>,
  pub workspace_menu_open: Signal<bool>,
  pub workspace_settings_open: Signal<bool>,
  pub account_menu_open: Signal<bool>,
  pub new_tab_menu_open: Signal<bool>,
  pub graph_modal_open: Signal<bool>,
  pub graph_data: Signal<PageGraphData>,
  pub version_modal_open: Signal<bool>,
  pub share_modal_open: Signal<bool>,
  pub transfer_modal_open: Signal<bool>,
  pub transfer_copy_mode: Signal<bool>,
  pub transfer_source_ids: Signal<Vec<Uuid>>,
  pub embedded_board_edit: Signal<Option<EmbeddedBoardEdit>>,
  pub update_status: Signal<String>,
  pub sidebar_rename_open: Signal<bool>,
  pub sidebar_rename_view: Signal<Option<Uuid>>,
  pub sidebar_rename_initial: Signal<String>,
  pub sidebar_emoji_open: Signal<bool>,
  pub sidebar_emoji_view: Signal<Option<Uuid>>,
  pub sidebar_emoji_current: Signal<Option<String>>,
}

impl ModalSignals {
  pub fn new(allow_local: bool) -> Self {
    Self {
      app_menu_open: signal(false),
      about_modal_open: signal(false),
      world_switcher_open: signal(!allow_local),
      world_can_close: signal(allow_local),
      search_modal_open: signal(false),
      search_initial_query: signal(None),
      search_open_in_new_tab: signal(false),
      settings_modal_open: signal(false),
      workspace_menu_open: signal(false),
      workspace_settings_open: signal(false),
      account_menu_open: signal(false),
      new_tab_menu_open: signal(false),
      graph_modal_open: signal(false),
      graph_data: signal(PageGraphData::default()),
      version_modal_open: signal(false),
      share_modal_open: signal(false),
      transfer_modal_open: signal(false),
      transfer_copy_mode: signal(false),
      transfer_source_ids: signal(Vec::new()),
      embedded_board_edit: signal(None),
      update_status: signal(String::new()),
      sidebar_rename_open: signal(false),
      sidebar_rename_view: signal(None),
      sidebar_rename_initial: signal(String::new()),
      sidebar_emoji_open: signal(false),
      sidebar_emoji_view: signal(None),
      sidebar_emoji_current: signal(None),
    }
  }
}

/// Signals backing the Global & Inline AI assistant dialogs.
#[derive(Clone, Copy)]
pub(crate) struct AiSignals {
  pub global_open: Signal<bool>,
  pub global_prompt: Signal<String>,
  pub global_target: Signal<AiTarget>,
  pub global_output: Signal<String>,
  pub global_streaming: Signal<bool>,
  pub global_done: Signal<bool>,
  pub global_error: Signal<Option<String>>,
  pub inline_open: Signal<bool>,
  pub inline_prompt: Signal<String>,
  pub inline_context: Signal<String>,
  pub inline_allow_replace: Signal<bool>,
  pub inline_output: Signal<String>,
  pub inline_phase: Signal<InlineAiPhase>,
  pub inline_error: Signal<Option<String>>,
}

impl AiSignals {
  pub fn new() -> Self {
    Self {
      global_open: signal(false),
      global_prompt: signal(String::new()),
      global_target: signal(AiTarget::NewPage),
      global_output: signal(String::new()),
      global_streaming: signal(false),
      global_done: signal(false),
      global_error: signal(None),
      inline_open: signal(false),
      inline_prompt: signal(String::new()),
      inline_context: signal(String::new()),
      inline_allow_replace: signal(true),
      inline_output: signal(String::new()),
      inline_phase: signal(InlineAiPhase::Idle),
      inline_error: signal(None),
    }
  }
}

/// Signals backing the public page Share modal.
#[derive(Clone, Copy)]
pub(crate) struct ShareModalSignals {
  pub loading: Signal<bool>,
  pub busy: Signal<bool>,
  pub enabled: Signal<bool>,
  pub token: Signal<Option<String>>,
  pub error: Signal<Option<String>>,
}

impl ShareModalSignals {
  pub fn new() -> Self {
    Self {
      loading: signal(false),
      busy: signal(false),
      enabled: signal(false),
      token: signal(None),
      error: signal(None),
    }
  }
}

/// Signals reflecting real-time CRDT sync & presence state for the active page.
#[derive(Clone, Copy)]
pub(crate) struct DocSyncSignals {
  pub phase: Signal<mica_sync::SyncPhase>,
  pub presence_users: Signal<Vec<mica_sync::PresenceUser>>,
  pub badge: Signal<String>,
  pub loading: Signal<bool>,
  pub load_error: Signal<Option<String>>,
}

impl DocSyncSignals {
  pub fn new() -> Self {
    Self {
      phase: signal(mica_sync::SyncPhase::Offline),
      presence_users: signal(Vec::new()),
      badge: signal(String::new()),
      loading: signal(false),
      load_error: signal(None),
    }
  }
}
