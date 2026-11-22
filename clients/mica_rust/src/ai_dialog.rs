//! Global `520px` `AiDialog` and Editor `480px` `InlineAiDialog` with 4 rewrite presets.
//!
//! 1:1 parity with `main:clients/mica_flutter/lib/ui/ai_dialog.dart` and
//! `main:clients/mica_flutter/lib/editor/inline_ai_dialog.dart`.

use std::rc::Rc;

use nsh_base::{button, input};
use nsh_core::{
  geometry::px,
  prelude::*,
};

use crate::{mica_icons, mica_theme::palette};

/// System prompt for in-place selection rewrites (data-safety rule: no preamble).
pub const K_AI_REWRITE_SYSTEM: &str = "You rewrite a fragment of a Markdown document in place. Output ONLY the \
rewritten Markdown — no preamble, no explanation, no code fence around the \
whole answer, no commentary before or after. Preserve the structure and \
formatting of the input (headings stay headings, list items stay list \
items, tables stay tables) unless the instruction asks otherwise. Keep the \
original language of the text unless asked to translate. If the instruction \
cannot be applied, return the input unchanged rather than explaining why.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiTarget {
  NewPage,
  CurrentPage,
  NewWorkspace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiEditDiffEntry {
  pub before: String,
  pub after: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AiRewritePreset {
  pub id: &'static str,
  pub label: &'static str,
  pub prompt: &'static str,
}

/// 4 built-in inline rewrite presets matching Flutter `app_zh.arb` 1:1.
pub const INLINE_AI_PRESETS: &[AiRewritePreset] = &[
  AiRewritePreset {
    id: "improve",
    label: "改进写作",
    prompt: "改进这段文字的表达，使其更清晰流畅，不改变原意。",
  },
  AiRewritePreset {
    id: "fix",
    label: "修正拼写语法",
    prompt: "修正其中的拼写、标点和语法错误，不做其它改动。",
  },
  AiRewritePreset {
    id: "shorter",
    label: "精简",
    prompt: "在保留全部要点的前提下把这段文字写得更短。",
  },
  AiRewritePreset {
    id: "longer",
    label: "扩写",
    prompt: "展开这段文字，补充必要的细节和说明。",
  },
];

/// State and callbacks for the `520px` Global `AiDialog`.
#[derive(Clone)]
pub struct GlobalAiState {
  pub open: Signal<bool>,
  pub can_edit: Signal<bool>,
  pub has_workspace: Signal<bool>,
  pub has_current_page: Signal<bool>,
  pub edit_mode: Signal<bool>,
  pub prompt: Signal<String>,
  pub target: Signal<AiTarget>,
  pub output: Signal<String>,
  pub edit_diffs: Signal<Vec<AiEditDiffEntry>>,
  pub streaming: Signal<bool>,
  pub done: Signal<bool>,
  pub applying: Signal<bool>,
  pub error: Signal<Option<String>>,
  pub on_generate: Rc<dyn Fn(String, AiTarget)>,
  pub on_apply: Rc<dyn Fn(String, AiTarget)>,
  pub on_close: Rc<dyn Fn()>,
}

impl GlobalAiState {
  pub fn is_busy(&self) -> bool {
    self.streaming.get() || self.applying.get()
  }

  pub fn is_busy_untracked(&self) -> bool {
    self.streaming.get_untracked() || self.applying.get_untracked()
  }

  /// Switches output target and resets any stale generated buffer/preview.
  pub fn select_target(&self, next: AiTarget) {
    if self.target.get_untracked() == next {
      return;
    }
    self.target.set(next);
    self.output.set(String::new());
    self.edit_diffs.set(Vec::new());
    self.done.set(false);
    self.error.set(None);
  }
}

/// Lifecycle for `InlineAiDialog`. Only `Succeeded` permits applying output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineAiPhase {
  Idle,
  Running,
  Succeeded,
  Failed,
  Cancelled,
}

/// State and callbacks for the `480px` Editor `InlineAiDialog`.
#[derive(Clone)]
pub struct InlineAiState {
  pub open: Signal<bool>,
  pub prompt: Signal<String>,
  pub context_text: Signal<String>,
  pub allow_replace: Signal<bool>,
  pub output: Signal<String>,
  pub phase: Signal<InlineAiPhase>,
  pub error: Signal<Option<String>>,
  pub on_generate: Rc<dyn Fn(String, String)>,
  pub on_insert_below: Rc<dyn Fn(String)>,
  pub on_replace_selection: Rc<dyn Fn(String)>,
  pub on_close: Rc<dyn Fn()>,
}

fn important_accent_hex(dark: bool) -> &'static str {
  if dark { "#a371f7" } else { "#8250df" }
}

/// Centered `520px` Global `AiDialog` modal (`问 AI`).
pub fn global_ai_dialog(state: GlobalAiState, dark_mode: Signal<bool>) -> impl IntoElement {
  when(move || state.open.get(), {
    let state = state.clone();
    move || {
      let st_bg = state.clone();
      let st_prompt = state.clone();
      let st_targets = state.clone();
      let st_out_cond = state.clone();
      let st_out_body = state.clone();
      let st_err_cond = state.clone();
      let st_err_body = state.clone();
      let st_actions = state.clone();

      view()
        .test_id("global-ai-backdrop")
        .absolute()
        .inset_0()
        .z_index(60)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("global-ai-backdrop-dismiss")
            .accessibility_label("关闭问 AI 弹窗")
            .absolute()
            .inset_0()
            .on_press(move || {
              if !st_bg.is_busy_untracked() {
                (st_bg.on_close)();
              }
            }),
        )
        .child(
          view()
            .test_id("global-ai-dialog")
            .w(px(520.0))
            .max_w(relative(0.92))
            .rounded(px(12.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .p(px(20.0))
            .gap(px(12.0))
            .shadow_md()
            .z_index(61)
            .child(
              // Title row: purple sparkle accent + 问 AI
              view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(dynamic(move || {
                  mica_icons::sparkles_icon(important_accent_hex(dark_mode.get()))
                }))
                .child(
                  text("问 AI")
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            )
            .child(
              // Prompt input
              input()
                .test_id("global-ai-prompt")
                .w_full()
                .min_h(px(72.0))
                .max_h(px(136.0))
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(move || palette(dark_mode.get()).border_normal)
                .bg(move || palette(dark_mode.get()).surface_base)
                .text_size(px(13.5))
                .text_color(move || palette(dark_mode.get()).text_primary)
                .multiline(true)
                .auto_focus(true)
                .readonly({
                  let st_ro = st_prompt.clone();
                  move || st_ro.is_busy()
                })
                .placeholder("例如：写一份项目启动计划，包含目标、里程碑和风险")
                .value({
                  let p = st_prompt.prompt;
                  move || p.get()
                })
                .on_change({
                  let prompt = st_prompt.prompt;
                  move |v: &str| prompt.set(v.to_owned())
                }),
            )
            .child(
              // Target segmented choices
              dynamic(move || {
                let p = palette(dark_mode.get());
                let can_edit = st_targets.can_edit.get();
                let has_ws = st_targets.has_workspace.get();
                let has_cur = st_targets.has_current_page.get();
                let selected = st_targets.target.get();
                let busy = st_targets.is_busy();

                let mut row = view().flex_row().flex_wrap().items_center().gap_2();
                if can_edit {
                  let st_np = st_targets.clone();
                  row = row.child(segment_choice_btn(
                    "ai-target-new-page",
                    "新建页面",
                    selected == AiTarget::NewPage,
                    !busy && has_ws,
                    dark_mode,
                    p,
                    move || st_np.select_target(AiTarget::NewPage),
                  ));
                }
                if can_edit && has_cur {
                  let st_cp = st_targets.clone();
                  row = row.child(segment_choice_btn(
                    "ai-target-current-page",
                    "当前页面",
                    selected == AiTarget::CurrentPage,
                    !busy,
                    dark_mode,
                    p,
                    move || st_cp.select_target(AiTarget::CurrentPage),
                  ));
                }
                let st_nw = st_targets.clone();
                row = row.child(segment_choice_btn(
                  "ai-target-new-workspace",
                  "新建工作区",
                  selected == AiTarget::NewWorkspace,
                  !busy,
                  dark_mode,
                  p,
                  move || st_nw.select_target(AiTarget::NewWorkspace),
                ));
                row
              }),
            )
            .child(when(
              move || {
                !st_out_cond.output.get().is_empty()
                  || st_out_cond.streaming.get()
                  || !st_out_cond.edit_diffs.get().is_empty()
              },
              move || {
                let st_box = st_out_body.clone();
                let st_status = st_out_body.clone();
                view()
                  .w_full()
                  .gap(px(6.0))
                  .child(
                    view()
                      .test_id("global-ai-output")
                      .w_full()
                      .h(px(220.0))
                      .p_3()
                      .rounded(px(8.0))
                      .border_1()
                      .border_color(move || palette(dark_mode.get()).border_normal)
                      .bg(move || palette(dark_mode.get()).surface_raised)
                      .child(
                        scroll().flex_1().min_h(px(0.0)).child(dynamic(move || {
                          let p = palette(dark_mode.get());
                          let is_edit = st_box.edit_mode.get()
                            && st_box.target.get() == AiTarget::CurrentPage;
                          let diffs = st_box.edit_diffs.get();
                          if is_edit && !diffs.is_empty() {
                            let mut col = view().w_full().gap_3();
                            for entry in diffs {
                              col = col.child(
                                view()
                                  .w_full()
                                  .gap_1()
                                  .child(
                                    text("原文")
                                      .text_size(px(12.0))
                                      .font_weight(FontWeight::MEDIUM)
                                      .text_color(p.text_muted),
                                  )
                                  .child(
                                    text(entry.before)
                                      .text_size(px(12.5))
                                      .text_color(p.text_primary),
                                  )
                                  .child(
                                    text("修改后")
                                      .text_size(px(12.0))
                                      .font_weight(FontWeight::MEDIUM)
                                      .text_color(p.text_muted),
                                  )
                                  .child(
                                    text(entry.after)
                                      .text_size(px(12.5))
                                      .text_color(p.text_primary),
                                  ),
                              );
                            }
                            col.into_element()
                          } else {
                            let body = st_box.output.get();
                            let display = if body.is_empty() {
                              "…".to_string()
                            } else {
                              body
                            };
                            text(display)
                              .text_size(px(13.0))
                              .text_color(p.text_primary)
                              .into_element()
                          }
                        })),
                      ),
                  )
                  .child(dynamic(move || {
                    let p = palette(dark_mode.get());
                    let is_edit = st_status.edit_mode.get()
                      && st_status.target.get() == AiTarget::CurrentPage;
                    if st_status.streaming.get() {
                      text("生成中…")
                        .text_size(px(12.0))
                        .text_color(p.text_muted)
                        .into_element()
                    } else if st_status.done.get() {
                      let msg = if is_edit {
                        "修改已生成，请检查后应用。"
                      } else {
                        "完成 —— 请检查后插入。"
                      };
                      text(msg)
                        .text_size(px(12.0))
                        .text_color(p.text_muted)
                        .into_element()
                    } else {
                      view().into_element()
                    }
                  }))
              },
            ))
            .child(when(move || st_err_cond.error.get().is_some(), move || {
              let err_sig = st_err_body.error;
              view()
                .test_id("global-ai-error")
                .w_full()
                .px_3()
                .py_2()
                .rounded(px(6.0))
                .bg(move || palette(dark_mode.get()).danger_wash)
                .child(
                  text(move || err_sig.get().unwrap_or_default())
                    .text_size(px(12.5))
                    .text_color(move || palette(dark_mode.get()).danger),
                )
            }))
            .child(
              // Footer actions
              dynamic(move || {
                let p = palette(dark_mode.get());
                let busy = st_actions.is_busy();
                let done = st_actions.done.get();
                let has_output = !st_actions.output.get().is_empty();
                let is_edit = st_actions.edit_mode.get()
                  && st_actions.target.get() == AiTarget::CurrentPage;

                let st_cancel = st_actions.clone();
                let mut row = view()
                  .w_full()
                  .pt_1()
                  .flex_row()
                  .items_center()
                  .justify_end()
                  .gap_2()
                  .child(ghost_action_btn(
                    "global-ai-cancel",
                    "取消",
                    !busy,
                    dark_mode,
                    p,
                    move || (st_cancel.on_close)(),
                  ));

                if !done {
                  let st_gen = st_actions.clone();
                  let label = if has_output { "重新生成" } else { "生成" };
                  row = row.child(primary_action_btn(
                    "global-ai-generate",
                    label,
                    !busy,
                    p,
                    move || {
                      let instr = st_gen.prompt.get_untracked().trim().to_string();
                      if !instr.is_empty() {
                        (st_gen.on_generate)(instr, st_gen.target.get_untracked());
                      }
                    },
                  ));
                } else {
                  let st_regen = st_actions.clone();
                  let st_apply = st_actions.clone();
                  let apply_label = if is_edit { "应用修改" } else { "插入" };
                  row = row
                    .child(ghost_action_btn(
                      "global-ai-regenerate",
                      "重新生成",
                      !busy,
                      dark_mode,
                      p,
                      move || {
                        let instr = st_regen.prompt.get_untracked().trim().to_string();
                        if !instr.is_empty() {
                          (st_regen.on_generate)(instr, st_regen.target.get_untracked());
                        }
                      },
                    ))
                    .child(primary_action_btn(
                      "global-ai-apply",
                      apply_label,
                      !busy,
                      p,
                      move || {
                        let out = st_apply.output.get_untracked().trim().to_string();
                        if !out.is_empty() {
                          (st_apply.on_apply)(out, st_apply.target.get_untracked());
                        }
                      },
                    ));
                }
                row
              }),
            ),
        )
    }
  })
}

/// Centered `480px` Editor `InlineAiDialog` modal (`问 AI`) with 4 rewrite presets.
pub fn inline_ai_dialog(state: InlineAiState, dark_mode: Signal<bool>) -> impl IntoElement {
  when(move || state.open.get(), {
    let state = state.clone();
    move || {
      let st_bg = state.clone();
      let st_chips = state.clone();
      let st_prompt = state.clone();
      let st_out_cond = state.clone();
      let st_out_body = state.clone();
      let st_err_cond = state.clone();
      let st_err_body = state.clone();
      let st_actions = state.clone();

      view()
        .test_id("inline-ai-backdrop")
        .absolute()
        .inset_0()
        .z_index(60)
        .items_center()
        .justify_center()
        .bg(move || palette(dark_mode.get()).backdrop)
        .child(
          button()
            .test_id("inline-ai-backdrop-dismiss")
            .accessibility_label("关闭行内问 AI 弹窗")
            .absolute()
            .inset_0()
            .on_press(move || {
              st_bg.phase.set(InlineAiPhase::Cancelled);
              (st_bg.on_close)();
            }),
        )
        .child(
          view()
            .test_id("inline-ai-dialog")
            .w(px(480.0))
            .max_w(relative(0.92))
            .rounded(px(12.0))
            .border_1()
            .border_color(move || palette(dark_mode.get()).border_normal)
            .bg(move || palette(dark_mode.get()).surface_overlay)
            .p(px(20.0))
            .gap(px(10.0))
            .shadow_md()
            .z_index(61)
            .child(
              // Header: purple sparkle accent + 问 AI
              view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(dynamic(move || {
                  mica_icons::sparkles_icon(important_accent_hex(dark_mode.get()))
                }))
                .child(
                  text("问 AI")
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            )
            .child(
              // 4 rewrite preset chips (fill the prompt field without firing immediately)
              dynamic(move || {
                let p = palette(dark_mode.get());
                let streaming = st_chips.phase.get() == InlineAiPhase::Running;
                let mut chips = view().flex_row().flex_wrap().items_center().gap(px(6.0));
                for preset in INLINE_AI_PRESETS {
                  let prompt_sig = st_chips.prompt;
                  let preset_prompt = preset.prompt;
                  chips = chips.child(
                    button()
                      .test_id(&format!("inline-ai-preset-{}", preset.id))
                      .accessibility_label(preset.label)
                      .px_2()
                      .py_1()
                      .rounded(px(6.0))
                      .border_1()
                      .border_color(p.border_normal)
                      .bg(p.surface_raised)
                      .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                      .on_press(move || {
                        if !streaming {
                          prompt_sig.set(preset_prompt.to_string());
                        }
                      })
                      .child(
                        text(preset.label)
                          .text_size(px(12.0))
                          .text_color(if streaming { p.text_ghost } else { p.text_primary }),
                      ),
                  );
                }
                chips
              }),
            )
            .child(
              // Prompt input
              input()
                .test_id("inline-ai-prompt")
                .w_full()
                .min_h(px(64.0))
                .max_h(px(120.0))
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .border_1()
                .border_color(move || palette(dark_mode.get()).border_normal)
                .bg(move || palette(dark_mode.get()).surface_base)
                .text_size(px(13.5))
                .text_color(move || palette(dark_mode.get()).text_primary)
                .multiline(true)
                .auto_focus(true)
                .readonly({
                  let phase = st_prompt.phase;
                  move || phase.get() == InlineAiPhase::Running
                })
                .placeholder("输入改写或生成指令…")
                .value({
                  let p = st_prompt.prompt;
                  move || p.get()
                })
                .on_change({
                  let prompt = st_prompt.prompt;
                  move |v: &str| prompt.set(v.to_owned())
                }),
            )
            .child(when(
              move || {
                !st_out_cond.output.get().is_empty()
                  || st_out_cond.phase.get() == InlineAiPhase::Running
              },
              move || {
                let output_sig = st_out_body.output;
                let phase_sig = st_out_body.phase;
                view()
                  .w_full()
                  .gap(px(6.0))
                  .child(
                    view()
                      .test_id("inline-ai-output")
                      .w_full()
                      .h(px(200.0))
                      .p_3()
                      .rounded(px(8.0))
                      .border_1()
                      .border_color(move || palette(dark_mode.get()).border_normal)
                      .bg(move || palette(dark_mode.get()).surface_raised)
                      .child(
                        scroll().flex_1().min_h(px(0.0)).child(
                          text(move || {
                            let s = output_sig.get();
                            if s.is_empty() { "…".to_string() } else { s }
                          })
                          .text_size(px(13.0))
                          .text_color(move || palette(dark_mode.get()).text_primary),
                        ),
                      ),
                  )
                  .child(when(move || phase_sig.get() == InlineAiPhase::Running, move || {
                    text("生成中…")
                      .text_size(px(12.0))
                      .text_color(move || palette(dark_mode.get()).text_muted)
                  }))
              },
            ))
            .child(when(move || st_err_cond.error.get().is_some(), move || {
              let err_sig = st_err_body.error;
              view()
                .test_id("inline-ai-error")
                .w_full()
                .px_3()
                .py_2()
                .rounded(px(6.0))
                .bg(move || palette(dark_mode.get()).danger_wash)
                .child(
                  text(move || err_sig.get().unwrap_or_default())
                    .text_size(px(12.5))
                    .text_color(move || palette(dark_mode.get()).danger),
                )
            }))
            .child(
              // Footer actions
              dynamic(move || {
                let p = palette(dark_mode.get());
                let phase = st_actions.phase.get();
                let streaming = phase == InlineAiPhase::Running;
                let applicable = phase == InlineAiPhase::Succeeded;
                let has_output = !st_actions.output.get().is_empty();
                let allow_replace = st_actions.allow_replace.get();

                let st_cancel = st_actions.clone();
                let mut row = view()
                  .w_full()
                  .pt_1()
                  .flex_row()
                  .items_center()
                  .justify_end()
                  .gap_2()
                  .child(ghost_action_btn(
                    "inline-ai-cancel",
                    "取消",
                    true,
                    dark_mode,
                    p,
                    move || {
                      st_cancel.phase.set(InlineAiPhase::Cancelled);
                      (st_cancel.on_close)();
                    },
                  ));

                if !applicable {
                  let st_gen = st_actions.clone();
                  let label = if has_output { "重新生成" } else { "生成" };
                  row = row.child(primary_action_btn(
                    "inline-ai-generate",
                    label,
                    !streaming,
                    p,
                    move || {
                      let instr = st_gen.prompt.get_untracked().trim().to_string();
                      if !instr.is_empty() {
                        (st_gen.on_generate)(instr, st_gen.context_text.get_untracked());
                      }
                    },
                  ));
                } else {
                  let st_regen = st_actions.clone();
                  let st_below = st_actions.clone();
                  let insert_label = if allow_replace { "插到下面" } else { "插入" };
                  row = row
                    .child(ghost_action_btn(
                      "inline-ai-regenerate",
                      "重新生成",
                      true,
                      dark_mode,
                      p,
                      move || {
                        let instr = st_regen.prompt.get_untracked().trim().to_string();
                        if !instr.is_empty() {
                          (st_regen.on_generate)(instr, st_regen.context_text.get_untracked());
                        }
                      },
                    ))
                    .child(ghost_action_btn(
                      "inline-ai-insert-below",
                      insert_label,
                      true,
                      dark_mode,
                      p,
                      move || {
                        let out = st_below.output.get_untracked();
                        if !out.is_empty() {
                          (st_below.on_insert_below)(out);
                        }
                      },
                    ));
                  if allow_replace {
                    let st_rep = st_actions.clone();
                    row = row.child(primary_action_btn(
                      "inline-ai-replace",
                      "替换所选",
                      true,
                      p,
                      move || {
                        let out = st_rep.output.get_untracked();
                        if !out.is_empty() {
                          (st_rep.on_replace_selection)(out);
                        }
                      },
                    ));
                  }
                }
                row
              }),
            ),
        )
    }
  })
}

fn segment_choice_btn(
  tid: &'static str,
  label: &'static str,
  selected: bool,
  enabled: bool,
  dark_mode: Signal<bool>,
  p: crate::mica_theme::MicaPalette,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .border_1()
    .border_color(if selected { p.border_strong } else { p.border_normal })
    .bg(if selected { p.surface_hover } else { p.surface_overlay })
    .hover(move |s| {
      if enabled {
        s.bg(palette(dark_mode.get_untracked()).surface_hover)
      } else {
        s
      }
    })
    .on_press(move || {
      if enabled {
        on_press();
      }
    })
    .child(
      text(label)
        .text_size(px(12.5))
        .font_weight(if selected { FontWeight::MEDIUM } else { FontWeight::NORMAL })
        .text_color(if !enabled {
          p.text_ghost
        } else if selected {
          p.text_primary
        } else {
          p.text_muted
        }),
    )
}

fn ghost_action_btn(
  tid: &'static str,
  label: &'static str,
  enabled: bool,
  dark_mode: Signal<bool>,
  p: crate::mica_theme::MicaPalette,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .hover(move |s| {
      if enabled {
        s.bg(palette(dark_mode.get_untracked()).surface_hover)
      } else {
        s
      }
    })
    .on_press(move || {
      if enabled {
        on_press();
      }
    })
    .child(
      text(label)
        .text_size(px(13.0))
        .text_color(if enabled { p.text_primary } else { p.text_ghost }),
    )
}

fn primary_action_btn(
  tid: &'static str,
  label: &'static str,
  enabled: bool,
  p: crate::mica_theme::MicaPalette,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .bg(if enabled { p.accent } else { p.surface_sunken })
    .on_press(move || {
      if enabled {
        on_press();
      }
    })
    .child(
      text(label)
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if enabled { p.on_primary } else { p.text_ghost }),
    )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn inline_ai_presets_have_four_chinese_entries() {
    assert_eq!(INLINE_AI_PRESETS.len(), 4);
    let labels: Vec<&str> = INLINE_AI_PRESETS.iter().map(|p| p.label).collect();
    assert_eq!(labels, vec!["改进写作", "修正拼写语法", "精简", "扩写"]);
    for preset in INLINE_AI_PRESETS {
      assert!(!preset.prompt.is_empty());
    }
  }
}
