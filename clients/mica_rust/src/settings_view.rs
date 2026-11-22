//! Two-column modal Settings dialog (`MicaSettingsShell` 1:1 restoration):
//! - Left 196px warm-gray category rail: 外观与语言 / 账号与云同步 / AI 助手 / 数据与导出 / 快捷键与关于.
//! - Right content pane with clean Notion-style setting cards, rendered as a centered
//!   floating modal over the active workspace (Esc or backdrop click closes cleanly).

use std::rc::Rc;

use nsh_base::{button, input};
use nsh_core::{geometry::px, prelude::*};

use crate::{mica_icons, mica_theme::palette};

const TAB_APPEARANCE: u8 = 0;
const TAB_ACCOUNT: u8 = 1;
const TAB_AI: u8 = 2;
const TAB_DATA: u8 = 3;
const TAB_ABOUT: u8 = 4;

pub struct SettingsCallbacks {
  pub dark_mode: Signal<bool>,
  pub locale_zh: Signal<bool>,
  pub server_url: Signal<String>,
  pub cloud_status: Signal<String>,
  pub export_preview: Signal<String>,
  pub on_toggle_theme: Rc<dyn Fn()>,
  pub on_toggle_locale: Rc<dyn Fn()>,
  pub on_check_server: Rc<dyn Fn(String)>,
  pub on_export_markdown: Rc<dyn Fn()>,
  pub on_export_html: Rc<dyn Fn()>,
  pub on_save_snapshot: Rc<dyn Fn()>,
  pub on_close: Rc<dyn Fn()>,
}

pub fn settings_modal_view(cb: Rc<SettingsCallbacks>) -> impl IntoElement {
  let active_tab = signal(TAB_APPEARANCE);
  let ai_preset = signal("deepseek".to_string());
  let ai_base_url = signal("https://api.deepseek.com".to_string());
  let ai_api_key = signal(String::new());
  let ai_saved_hint = signal(String::new());

  let dark_mode = cb.dark_mode;
  let close_bg = cb.on_close.clone();
  let close_btn = cb.on_close.clone();
  let cb_body = cb.clone();

  view()
    .test_id("settings-modal-backdrop")
    .absolute()
    .inset_0()
    .z_index(50)
    .items_center()
    .justify_center()
    .bg(move || palette(dark_mode.get()).backdrop)
    .child(
      button()
        .test_id("settings-backdrop-dismiss")
        .accessibility_label("关闭设置弹窗")
        .absolute()
        .inset_0()
        .on_press(move || close_bg()),
    )
    .child(
      // Centered two-column Settings dialog (780×540)
      view()
        .test_id("settings-panel")
        .w(px(780.0))
        .h(px(540.0))
        .flex_row()
        .rounded(px(12.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .overflow_hidden()
        .z_index(51)
        .child(
          // Left navigation rail (196px, warm gray)
          view()
            .w(px(196.0))
            .h_full()
            .flex_shrink_0()
            .p_3()
            .gap_1()
            .border_r_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .bg(move || palette(dark_mode.get()).surface_raised)
            .child(
              view().px_2().pb_2().child(
                text("设置")
                  .text_size(px(14.0))
                  .font_weight(FontWeight::SEMIBOLD)
                  .text_color(move || palette(dark_mode.get()).text_primary),
              ),
            )
            .child(nav_item(
              "settings-tab-appearance",
              "外观与语言",
              TAB_APPEARANCE,
              active_tab,
              dark_mode,
              mica_icons::sun_icon,
            ))
            .child(nav_item(
              "settings-tab-account",
              "账号与云同步",
              TAB_ACCOUNT,
              active_tab,
              dark_mode,
              mica_icons::cloud,
            ))
            .child(nav_item(
              "settings-tab-ai",
              "AI 助手",
              TAB_AI,
              active_tab,
              dark_mode,
              mica_icons::sparkles_icon,
            ))
            .child(nav_item(
              "settings-tab-data",
              "数据与导出",
              TAB_DATA,
              active_tab,
              dark_mode,
              mica_icons::database_icon,
            ))
            .child(nav_item(
              "settings-tab-about",
              "快捷键与关于",
              TAB_ABOUT,
              active_tab,
              dark_mode,
              mica_icons::keyboard_icon,
            )),
        )
        .child(
          // Right content pane
          view()
            .flex_1()
            .h_full()
            .bg(move || palette(dark_mode.get()).surface_base)
            .child(
              // Dialog top bar
              view()
                .h(px(48.0))
                .px_5()
                .flex_row()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(move || palette(dark_mode.get()).border_subtle)
                .child(
                  text(move || match active_tab.get() {
                    TAB_APPEARANCE => "外观与语言",
                    TAB_ACCOUNT => "账号与云同步",
                    TAB_AI => "AI 助手",
                    TAB_DATA => "数据与导入导出",
                    _ => "快捷键与关于",
                  })
                  .text_size(px(15.0))
                  .font_weight(FontWeight::SEMIBOLD)
                  .text_color(move || palette(dark_mode.get()).text_primary),
                )
                .child(
                  button()
                    .test_id("settings-close-btn")
                    .accessibility_label("关闭设置")
                    .w(px(28.0))
                    .h(px(28.0))
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                    .on_press(move || close_btn())
                    .child(dynamic(move || {
                      mica_icons::close_sm(palette(dark_mode.get()).text_faint_hex)
                    })),
                ),
            )
            .child(
              view()
                .flex_1()
                .p_5()
                .gap_4()
                .child(when(move || active_tab.get() == TAB_APPEARANCE, {
                  let cb_a = cb_body.clone();
                  move || appearance_tab(cb_a.clone())
                }))
                .child(when(move || active_tab.get() == TAB_ACCOUNT, {
                  let cb_c = cb_body.clone();
                  move || account_sync_tab(cb_c.clone())
                }))
                .child(when(move || active_tab.get() == TAB_AI, move || {
                  ai_tab(dark_mode, ai_preset, ai_base_url, ai_api_key, ai_saved_hint)
                }))
                .child(when(move || active_tab.get() == TAB_DATA, {
                  let cb_d = cb_body.clone();
                  move || data_export_tab(cb_d.clone())
                }))
                .child(when(move || active_tab.get() == TAB_ABOUT, move || {
                  about_shortcuts_tab(dark_mode)
                })),
            ),
        ),
    )
}

fn nav_item<F, I>(
  tid: &'static str,
  label: &'static str,
  tab: u8,
  active_tab: Signal<u8>,
  dark_mode: Signal<bool>,
  icon_fn: F,
) -> impl IntoElement
where
  F: Fn(&'static str) -> I + 'static,
  I: IntoElement + 'static,
{
  button()
    .test_id(tid)
    .accessibility_label(label)
    .w_full()
    .h(px(34.0))
    .px_2()
    .flex_row()
    .items_center()
    .gap_2()
    .rounded(px(6.0))
    .bg(move || {
      let p = palette(dark_mode.get());
      if active_tab.get() == tab {
        p.surface_hover
      } else {
        p.surface_raised
      }
    })
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(move || active_tab.set(tab))
    .child(dynamic(move || {
      let p = palette(dark_mode.get());
      icon_fn(if active_tab.get() == tab {
        p.text_primary_hex
      } else {
        p.text_faint_hex
      })
    }))
    .child(
      text(label)
        .text_size(px(13.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(move || {
          let p = palette(dark_mode.get());
          if active_tab.get() == tab {
            p.text_primary
          } else {
            p.text_muted
          }
        }),
    )
}

fn appearance_tab(cb: Rc<SettingsCallbacks>) -> impl IntoElement {
  let dark_mode = cb.dark_mode;
  let locale_zh = cb.locale_zh;
  let on_theme = cb.on_toggle_theme.clone();
  let on_locale = cb.on_toggle_locale.clone();

  view()
    .gap_3()
    .child(
      setting_card(dark_mode)
        .child(
          view()
            .flex_row()
            .items_center()
            .justify_between()
            .child(
              view()
                .gap_1()
                .child(
                  text("主题外观")
                    .text_size(px(13.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                )
                .child(
                  text("在暖白浅色纸面与低饱和深色模式之间切换")
                    .text_size(px(12.0))
                    .text_color(move || palette(dark_mode.get()).text_muted),
                ),
            )
            .child(
              button()
                .test_id("settings-toggle-theme")
                .accessibility_label("切换浅色或深色主题")
                .px_3()
                .py_1()
                .rounded(px(6.0))
                .border_1()
                .border_color(move || palette(dark_mode.get()).border_normal)
                .bg(move || palette(dark_mode.get()).surface_sunken)
                .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                .on_press(move || on_theme())
                .child(
                  text(move || {
                    if dark_mode.get() {
                      "当前：深色模式"
                    } else {
                      "当前：浅色模式"
                    }
                  })
                  .text_size(px(12.5))
                  .text_color(move || palette(dark_mode.get()).text_primary),
                ),
            ),
        ),
    )
    .child(
      setting_card(dark_mode).child(
        view()
          .flex_row()
          .items_center()
          .justify_between()
          .child(
            view()
              .gap_1()
              .child(
                text("界面语言")
                  .text_size(px(13.5))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(move || palette(dark_mode.get()).text_primary),
              )
              .child(
                text("切换 Mica 界面显示语言（中文 / English）")
                  .text_size(px(12.0))
                  .text_color(move || palette(dark_mode.get()).text_muted),
              ),
          )
          .child(
            button()
              .test_id("settings-toggle-locale")
              .accessibility_label("切换界面语言")
              .px_3()
              .py_1()
              .rounded(px(6.0))
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .bg(move || palette(dark_mode.get()).surface_sunken)
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || on_locale())
              .child(
                text(move || {
                  if locale_zh.get() {
                    "语言：简体中文"
                  } else {
                    "Language: English"
                  }
                })
                .text_size(px(12.5))
                .text_color(move || palette(dark_mode.get()).text_primary),
              ),
          ),
      ),
    )
}

fn account_sync_tab(cb: Rc<SettingsCallbacks>) -> impl IntoElement {
  let dark_mode = cb.dark_mode;
  let server_url = cb.server_url;
  let cloud_status = cb.cloud_status;
  let on_check = cb.on_check_server.clone();

  view().gap_3().child(
    setting_card(dark_mode)
      .gap_3()
      .child(
        text("云端同步服务器")
          .text_size(px(13.5))
          .font_weight(FontWeight::MEDIUM)
          .text_color(move || palette(dark_mode.get()).text_primary),
      )
      .child(
        view()
          .flex_row()
          .items_center()
          .gap_2()
          .child(
            input()
              .test_id("settings-server-url")
              .placeholder("https://your-mica-server.example.com")
              .flex_1()
              .px_3()
              .py_2()
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .rounded(px(6.0))
              .text_size(px(13.0))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .bg(move || palette(dark_mode.get()).surface_base)
              .value(move || server_url.get())
              .on_change(move |v: &str| server_url.set(v.to_owned())),
          )
          .child(
            button()
              .test_id("settings-check-server")
              .accessibility_label("检查云端服务器连接")
              .px_3()
              .py_2()
              .rounded(px(6.0))
              .bg(move || palette(dark_mode.get()).accent)
              .on_press(move || on_check(server_url.get_untracked()))
              .child(
                text("检测连接")
                  .text_size(px(12.5))
                  .font_weight(FontWeight::MEDIUM)
                  .text_color(nsh_core::color::white()),
              ),
          ),
      )
      .child(
        text(move || cloud_status.get())
          .text_size(px(12.0))
          .text_color(move || palette(dark_mode.get()).text_muted),
      ),
  )
}

fn ai_tab(
  dark_mode: Signal<bool>,
  ai_preset: Signal<String>,
  ai_base_url: Signal<String>,
  ai_api_key: Signal<String>,
  ai_saved_hint: Signal<String>,
) -> impl IntoElement {
  let presets: &[(&str, &str, &str)] = &[
    ("deepseek", "DeepSeek", "https://api.deepseek.com"),
    ("zhipu", "智谱 GLM", "https://open.bigmodel.cn/api/coding/paas/v4"),
    ("kimi", "Kimi", "https://api.moonshot.cn/v1"),
    ("openai", "OpenAI", "https://api.openai.com/v1"),
    ("anthropic", "Claude", "https://api.anthropic.com"),
    ("custom", "自定义", "http://127.0.0.1:11434/v1"),
  ];

  let mut chips = view().flex_row().flex_wrap().gap_1();
  for &(id, label, url) in presets {
    chips = chips.child(
      button()
        .test_id(&format!("ai-preset-{id}"))
        .accessibility_label(format!("选择 AI 服务商 {label}"))
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || {
          let p = palette(dark_mode.get());
          if ai_preset.get() == id {
            p.surface_hover
          } else {
            p.surface_base
          }
        })
        .on_press(move || {
          ai_preset.set(id.to_string());
          ai_base_url.set(url.to_string());
        })
        .child(
          text(label)
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_primary),
        ),
    );
  }

  view().gap_3().child(
    setting_card(dark_mode)
      .gap_3()
      .child(
        text("模型服务商预设")
          .text_size(px(13.5))
          .font_weight(FontWeight::MEDIUM)
          .text_color(move || palette(dark_mode.get()).text_primary),
      )
      .child(chips)
      .child(
        view()
          .gap_1()
          .child(
            text("API Base URL")
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_muted),
          )
          .child(
            input()
              .test_id("ai-base-url-input")
              .w_full()
              .px_3()
              .py_2()
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .rounded(px(6.0))
              .text_size(px(13.0))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .value(move || ai_base_url.get())
              .on_change(move |v: &str| ai_base_url.set(v.to_owned())),
          ),
      )
      .child(
        view()
          .gap_1()
          .child(
            text("API Key")
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_muted),
          )
          .child(
            input()
              .test_id("ai-api-key-input")
              .placeholder("sk-...")
              .w_full()
              .px_3()
              .py_2()
              .border_1()
              .border_color(move || palette(dark_mode.get()).border_normal)
              .rounded(px(6.0))
              .text_size(px(13.0))
              .text_color(move || palette(dark_mode.get()).text_primary)
              .value(move || ai_api_key.get())
              .on_change(move |v: &str| ai_api_key.set(v.to_owned())),
          ),
      )
      .child(
        view()
          .flex_row()
          .items_center()
          .gap_2()
          .child(
            button()
              .test_id("ai-save-btn")
              .accessibility_label("保存 AI 配置")
              .px_3()
              .py_1()
              .rounded(px(6.0))
              .bg(move || palette(dark_mode.get()).accent)
              .on_press(move || {
                ai_saved_hint.set(format!(
                  "已保存 {} 配置（{}）",
                  ai_preset.get_untracked(),
                  ai_base_url.get_untracked()
                ));
              })
              .child(
                text("保存配置")
                  .text_size(px(12.5))
                  .text_color(nsh_core::color::white()),
              ),
          )
          .child(
            text(move || ai_saved_hint.get())
              .text_size(px(12.0))
              .text_color(move || palette(dark_mode.get()).text_muted),
          ),
      ),
  )
}

fn data_export_tab(cb: Rc<SettingsCallbacks>) -> impl IntoElement {
  let dark_mode = cb.dark_mode;
  let on_md = cb.on_export_markdown.clone();
  let on_html = cb.on_export_html.clone();
  let on_snap = cb.on_save_snapshot.clone();
  let export_preview = cb.export_preview;

  view().gap_3().child(
    setting_card(dark_mode)
      .gap_3()
      .child(
        text("文档导出与版本历史（单一 Rust 引擎）")
          .text_size(px(13.5))
          .font_weight(FontWeight::MEDIUM)
          .text_color(move || palette(dark_mode.get()).text_primary),
      )
      .child(
        view()
          .flex_row()
          .flex_wrap()
          .gap_2()
          .child(action_pill(
            "settings-export-md",
            "导出 Markdown",
            dark_mode,
            move || on_md(),
          ))
          .child(action_pill(
            "settings-export-html",
            "导出 HTML",
            dark_mode,
            move || on_html(),
          ))
          .child(action_pill(
            "settings-save-snapshot",
            "保存版本快照",
            dark_mode,
            move || on_snap(),
          )),
      )
      .child(
        text(move || {
          let preview = export_preview.get();
          if preview.is_empty() {
            "本地数据以 SQLite Vault 实时落盘，离线可用。".to_string()
          } else {
            preview
          }
        })
        .text_size(px(12.0))
        .text_color(move || palette(dark_mode.get()).text_muted),
      ),
  )
}

fn about_shortcuts_tab(dark_mode: Signal<bool>) -> impl IntoElement {
  let rows: &[(&str, &str)] = &[
    ("Ctrl+Shift+F", "搜索工作区"),
    ("Ctrl+,", "打开设置"),
    ("Ctrl+A", "全选块 / 再按一次全选整篇文档"),
    ("Ctrl+Z / Ctrl+Y", "撤销 / 重做（CRDT 原生历史）"),
    ("Ctrl+B / Ctrl+I / Ctrl+E", "粗体 / 斜体 / 行内代码"),
    ("Enter / Backspace", "新建段落块 / 行首合并上一块"),
    ("↑ / ↓", "跨块上下切换焦点"),
    ("/", "打开斜杠命令插入菜单"),
    ("Esc", "关闭当前弹窗或清除全选"),
  ];

  let mut list = setting_card(dark_mode).gap_2().child(
    text("常用快捷键")
      .text_size(px(13.5))
      .font_weight(FontWeight::MEDIUM)
      .text_color(move || palette(dark_mode.get()).text_primary),
  );
  for &(k, desc) in rows {
    list = list.child(
      view()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
          text(desc)
            .text_size(px(12.5))
            .text_color(move || palette(dark_mode.get()).text_muted),
        )
        .child(
          view()
            .px_2()
            .py_1()
            .rounded(px(4.0))
            .bg(move || palette(dark_mode.get()).surface_sunken)
            .child(
              text(k)
                .text_size(px(11.5))
                .text_color(move || palette(dark_mode.get()).text_primary),
            ),
        ),
    );
  }

  view().gap_3().child(list).child(
    setting_card(dark_mode).child(
      text(format!(
        "Mica v{} · 100% 全栈纯 Rust（mica-core + mica-editor + mica-board + nsh 原生壳）",
        env!("CARGO_PKG_VERSION")
      ))
      .text_size(px(12.0))
      .text_color(move || palette(dark_mode.get()).text_faint),
    ),
  )
}

fn setting_card(dark_mode: Signal<bool>) -> View {
  view()
    .w_full()
    .p_4()
    .rounded(px(8.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_raised)
}

fn action_pill(
  tid: &'static str,
  label: &'static str,
  dark_mode: Signal<bool>,
  on_click: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .accessibility_label(label)
    .px_3()
    .py_1()
    .rounded(px(6.0))
    .border_1()
    .border_color(move || palette(dark_mode.get()).border_normal)
    .bg(move || palette(dark_mode.get()).surface_base)
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_click)
    .child(
      text(label)
        .text_size(px(12.5))
        .text_color(move || palette(dark_mode.get()).text_primary),
    )
}
