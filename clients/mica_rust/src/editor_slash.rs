//! Slash command palette (`/`) for Mica's Route 2-B pure-Rust editor.

use std::rc::Rc;

use mica_editor::EditorCommand;
use nsh_base::{button, theme};
use nsh_core::prelude::*;

/// One entry in the slash-command insertion menu.
#[derive(Clone, Copy)]
pub struct SlashItem {
  pub id: &'static str,
  pub label: &'static str,
  pub keywords: &'static str,
  pub kind: &'static str,
}

pub const SLASH_ITEMS: &[SlashItem] = &[
  SlashItem {
    id: "slash-h1",
    label: "Heading 1",
    keywords: "h1 heading title 标题",
    kind: "heading1",
  },
  SlashItem {
    id: "slash-h2",
    label: "Heading 2",
    keywords: "h2 heading subtitle 二级标题",
    kind: "heading2",
  },
  SlashItem {
    id: "slash-h3",
    label: "Heading 3",
    keywords: "h3 heading 三级标题",
    kind: "heading3",
  },
  SlashItem {
    id: "slash-bullet",
    label: "Bulleted List",
    keywords: "bullet list ul 列表",
    kind: "bulleted_list",
  },
  SlashItem {
    id: "slash-ordered",
    label: "Numbered List",
    keywords: "numbered ordered ol 编号",
    kind: "numbered_list",
  },
  SlashItem {
    id: "slash-todo",
    label: "Todo Checklist",
    keywords: "todo task check 待办",
    kind: "todo",
  },
  SlashItem {
    id: "slash-quote",
    label: "Blockquote",
    keywords: "quote cite 引用",
    kind: "quote",
  },
  SlashItem {
    id: "slash-code",
    label: "Code Block",
    keywords: "code fence rust 代码",
    kind: "code_block",
  },
  SlashItem {
    id: "slash-table",
    label: "Table (2×2)",
    keywords: "table grid 表格",
    kind: "table",
  },
  SlashItem {
    id: "slash-math",
    label: "Math Formula (RaTeX)",
    keywords: "math latex formula 公式",
    kind: "math",
  },
  SlashItem {
    id: "slash-mermaid",
    label: "Mermaid Diagram",
    keywords: "mermaid flowchart graph 图表",
    kind: "mermaid",
  },
  SlashItem {
    id: "slash-board",
    label: "Embedded Whiteboard",
    keywords: "board whiteboard mindmap 画板 导图",
    kind: "board",
  },
  SlashItem {
    id: "slash-divider",
    label: "Divider",
    keywords: "divider hr line 分隔线",
    kind: "divider",
  },
];

/// Converts a slash item selection into an L2 `EditorCommand`.
pub fn command_for_slash(item_kind: &str) -> EditorCommand {
  match item_kind {
    "heading1" => EditorCommand::TransformBlock {
      kind: "heading".into(),
      data: Some(serde_json::json!({ "level": 1 })),
    },
    "heading2" => EditorCommand::TransformBlock {
      kind: "heading".into(),
      data: Some(serde_json::json!({ "level": 2 })),
    },
    "heading3" => EditorCommand::TransformBlock {
      kind: "heading".into(),
      data: Some(serde_json::json!({ "level": 3 })),
    },
    "bulleted_list" => EditorCommand::TransformBlock {
      kind: "bulleted_list".into(),
      data: None,
    },
    "numbered_list" => EditorCommand::TransformBlock {
      kind: "numbered_list".into(),
      data: None,
    },
    "todo" => EditorCommand::TransformBlock {
      kind: "todo".into(),
      data: Some(serde_json::json!({ "checked": false })),
    },
    "quote" => EditorCommand::TransformBlock {
      kind: "quote".into(),
      data: None,
    },
    "code_block" => EditorCommand::TransformBlock {
      kind: "code_block".into(),
      data: Some(serde_json::json!({ "language": "rust" })),
    },
    "table" => EditorCommand::TransformBlock {
      kind: "table".into(),
      data: Some(serde_json::json!({
        "rows": [["Column A", "Column B"], ["Cell 1", "Cell 2"]],
        "align": ["left", "left"]
      })),
    },
    "math" => EditorCommand::TransformBlock {
      kind: "math".into(),
      data: Some(serde_json::json!({})),
    },
    "mermaid" => EditorCommand::TransformBlock {
      kind: "code_block".into(),
      data: Some(serde_json::json!({ "language": "mermaid" })),
    },
    "board" => EditorCommand::TransformBlock {
      kind: "code_block".into(),
      data: Some(serde_json::json!({ "language": "mica-board" })),
    },
    "divider" => EditorCommand::InsertDivider,
    _ => EditorCommand::TransformBlock {
      kind: "paragraph".into(),
      data: None,
    },
  }
}

/// Renders the interactive slash-menu strip when open.
pub fn slash_menu_view(
  query: String,
  on_pick: Rc<dyn Fn(&'static str)>,
  on_close: Rc<dyn Fn()>,
) -> impl IntoElement {
  let q = query.trim().trim_start_matches('/').to_lowercase();
  let mut panel = view()
    .test_id("slash-menu")
    .flex_row()
    .flex_wrap()
    .items_center()
    .gap_1()
    .px_4()
    .py_2()
    .border_b_1()
    .border_color(move || theme().border)
    .bg(move || theme().popover)
    .child(
      text("Insert (/):")
        .text_xs()
        .text_color(move || theme().muted_foreground),
    );

  for item in SLASH_ITEMS {
    if !q.is_empty()
      && !item.label.to_lowercase().contains(&q)
      && !item.keywords.to_lowercase().contains(&q)
    {
      continue;
    }
    let pick = on_pick.clone();
    let kind = item.kind;
    panel = panel.child(
      button()
        .test_id(item.id)
        .px_2()
        .py_1()
        .border_1()
        .border_color(move || theme().border)
        .rounded(move || theme().radius)
        .bg(move || theme().secondary)
        .on_press(move || pick(kind))
        .child(
          text(item.label)
            .text_xs()
            .text_color(move || theme().secondary_foreground),
        ),
    );
  }

  panel.child(
    button()
      .test_id("slash-close")
      .px_2()
      .py_1()
      .on_press(move || on_close())
      .child(
        text("✕")
          .text_xs()
          .text_color(move || theme().muted_foreground),
      ),
  )
}
