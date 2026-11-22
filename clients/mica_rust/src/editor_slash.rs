//! Slash command palette (`/`) catalog for Mica's pure-Rust `RichEditor`.

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
    label: "一级标题",
    keywords: "h1 heading title 标题",
    kind: "heading1",
  },
  SlashItem {
    id: "slash-h2",
    label: "二级标题",
    keywords: "h2 heading subtitle 二级标题",
    kind: "heading2",
  },
  SlashItem {
    id: "slash-h3",
    label: "三级标题",
    keywords: "h3 heading 三级标题",
    kind: "heading3",
  },
  SlashItem {
    id: "slash-bullet",
    label: "无序列表",
    keywords: "bullet list ul 列表",
    kind: "bulleted_list",
  },
  SlashItem {
    id: "slash-ordered",
    label: "有序列表",
    keywords: "numbered ordered ol 编号",
    kind: "numbered_list",
  },
  SlashItem {
    id: "slash-todo",
    label: "待办清单",
    keywords: "todo task check 待办",
    kind: "todo",
  },
  SlashItem {
    id: "slash-quote",
    label: "引用块",
    keywords: "quote cite 引用",
    kind: "quote",
  },
  SlashItem {
    id: "slash-code",
    label: "代码块",
    keywords: "code fence rust 代码",
    kind: "code_block",
  },
  SlashItem {
    id: "slash-table",
    label: "表格 (2×2)",
    keywords: "table grid 表格",
    kind: "table",
  },
  SlashItem {
    id: "slash-math",
    label: "数学公式 (RaTeX)",
    keywords: "math latex formula 公式",
    kind: "math",
  },
  SlashItem {
    id: "slash-mermaid",
    label: "Mermaid 图表",
    keywords: "mermaid flowchart graph 图表",
    kind: "mermaid",
  },
  SlashItem {
    id: "slash-board",
    label: "内嵌白板",
    keywords: "board whiteboard mindmap 画板 导图",
    kind: "board",
  },
  SlashItem {
    id: "slash-divider",
    label: "分割线",
    keywords: "divider hr line 分隔线",
    kind: "divider",
  },
];
