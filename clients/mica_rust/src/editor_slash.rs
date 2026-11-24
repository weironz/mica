//! Slash command palette (`/`) catalog for Mica's pure-Rust `RichEditor`.
//! Exact 22-item 1:1 parity with `main:clients/mica_flutter/lib/editor/editor_dialogs.dart`
//! and `editor.dart` (`_slashOptions` + `_aiSlashOption`).

/// One entry in the caret-anchored slash-command insertion menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlashItem {
  pub id: &'static str,
  pub label: &'static str,
  pub en_label: &'static str,
  pub kind: &'static str,
  pub level: Option<u64>,
  pub alert: Option<&'static str>,
  pub template: Option<&'static str>,
  pub icon_key: &'static str,
}

pub const SLASH_ITEMS: &[SlashItem] = &[
  SlashItem {
    id: "slash-ai",
    label: "问 AI",
    en_label: "Ask AI",
    kind: "__ai__",
    level: None,
    alert: None,
    template: None,
    icon_key: "sparkles",
  },
  SlashItem {
    id: "slash-text",
    label: "正文",
    en_label: "Text",
    kind: "paragraph",
    level: None,
    alert: None,
    template: None,
    icon_key: "text",
  },
  SlashItem {
    id: "slash-h1",
    label: "一级标题",
    en_label: "Heading 1",
    kind: "heading",
    level: Some(1),
    alert: None,
    template: None,
    icon_key: "h1",
  },
  SlashItem {
    id: "slash-h2",
    label: "二级标题",
    en_label: "Heading 2",
    kind: "heading",
    level: Some(2),
    alert: None,
    template: None,
    icon_key: "h2",
  },
  SlashItem {
    id: "slash-h3",
    label: "三级标题",
    en_label: "Heading 3",
    kind: "heading",
    level: Some(3),
    alert: None,
    template: None,
    icon_key: "h3",
  },
  SlashItem {
    id: "slash-bullet",
    label: "无序列表",
    en_label: "Bulleted list",
    kind: "bulleted_list",
    level: None,
    alert: None,
    template: None,
    icon_key: "bullet",
  },
  SlashItem {
    id: "slash-ordered",
    label: "有序列表",
    en_label: "Numbered list",
    kind: "numbered_list",
    level: None,
    alert: None,
    template: None,
    icon_key: "ordered",
  },
  SlashItem {
    id: "slash-todo",
    label: "待办",
    en_label: "To-do",
    kind: "todo",
    level: None,
    alert: None,
    template: None,
    icon_key: "todo",
  },
  SlashItem {
    id: "slash-quote",
    label: "引用",
    en_label: "Quote",
    kind: "quote",
    level: None,
    alert: None,
    template: None,
    icon_key: "quote",
  },
  SlashItem {
    id: "slash-alert-note",
    label: "标注:注意",
    en_label: "Note",
    kind: "quote",
    level: None,
    alert: Some("note"),
    template: None,
    icon_key: "alert_note",
  },
  SlashItem {
    id: "slash-alert-tip",
    label: "标注:提示",
    en_label: "Tip",
    kind: "quote",
    level: None,
    alert: Some("tip"),
    template: None,
    icon_key: "alert_tip",
  },
  SlashItem {
    id: "slash-alert-important",
    label: "标注:重要",
    en_label: "Important",
    kind: "quote",
    level: None,
    alert: Some("important"),
    template: None,
    icon_key: "alert_important",
  },
  SlashItem {
    id: "slash-alert-warning",
    label: "标注:警告",
    en_label: "Warning",
    kind: "quote",
    level: None,
    alert: Some("warning"),
    template: None,
    icon_key: "alert_warning",
  },
  SlashItem {
    id: "slash-alert-caution",
    label: "标注:小心",
    en_label: "Caution",
    kind: "quote",
    level: None,
    alert: Some("caution"),
    template: None,
    icon_key: "alert_caution",
  },
  SlashItem {
    id: "slash-code",
    label: "代码",
    en_label: "Code",
    kind: "code_block",
    level: None,
    alert: None,
    template: None,
    icon_key: "code",
  },
  SlashItem {
    id: "slash-math",
    label: "数学公式",
    en_label: "Math formula",
    kind: "math",
    level: None,
    alert: None,
    template: None,
    icon_key: "math",
  },
  SlashItem {
    id: "slash-whiteboard",
    label: "画板",
    en_label: "Whiteboard",
    kind: "board",
    level: None,
    alert: None,
    template: Some("whiteboard"),
    icon_key: "whiteboard",
  },
  SlashItem {
    id: "slash-mindmap",
    label: "思维导图",
    en_label: "Mind map",
    kind: "board",
    level: None,
    alert: None,
    template: Some("mindmap"),
    icon_key: "mindmap",
  },
  SlashItem {
    id: "slash-flowchart",
    label: "流程图",
    en_label: "Flowchart",
    kind: "board",
    level: None,
    alert: None,
    template: Some("flowchart"),
    icon_key: "flowchart",
  },
  SlashItem {
    id: "slash-table",
    label: "表格",
    en_label: "Table",
    kind: "table",
    level: None,
    alert: None,
    template: None,
    icon_key: "table",
  },
  SlashItem {
    id: "slash-divider",
    label: "分割线",
    en_label: "Divider",
    kind: "divider",
    level: None,
    alert: None,
    template: None,
    icon_key: "divider",
  },
  SlashItem {
    id: "slash-image",
    label: "图片",
    en_label: "Image",
    kind: "image",
    level: None,
    alert: None,
    template: None,
    icon_key: "image",
  },
];

/// Filters slash menu items by Chinese label, English label, `kind`, `alert`,
/// `template`, or heading shorthand (`h1`/`h2`/`h3`), case-insensitively.
pub fn filter_slash_items(query: &str, ai_enabled: bool) -> Vec<&'static SlashItem> {
  let q = query.trim_start_matches('/').to_lowercase();
  SLASH_ITEMS
    .iter()
    .filter(|item| {
      if item.kind == "__ai__" && !ai_enabled {
        return false;
      }
      if q.is_empty() {
        return true;
      }
      item.label.to_lowercase().contains(&q)
        || item.en_label.to_lowercase().contains(&q)
        || item.kind.to_lowercase().contains(&q)
        || item.alert.is_some_and(|a| a.to_lowercase().contains(&q))
        || item.template.is_some_and(|t| t.to_lowercase().contains(&q))
        || item.level.is_some_and(|lvl| format!("h{lvl}").contains(&q))
    })
    .collect()
}

/// Extracts an active `/` token ending at the caret inside `before_caret`.
///
/// Mirrors Flutter `_refreshSlash`:
/// - `/` must be at block start (`index == 0`) or immediately preceded by `' '` or `'\n'`.
/// - A leading space after `/` (`"/ ..."`) is prose, not a command, and returns `None`.
/// - Returns `(slash_byte_index, query_after_slash)`.
pub fn extract_slash_query(before_caret: &str) -> Option<(usize, &str)> {
  let slash = before_caret.rfind('/')?;
  if slash > 0 {
    let prev = before_caret[..slash].chars().next_back()?;
    if prev != ' ' && prev != '\n' {
      return None;
    }
  }
  let query = &before_caret[slash + 1..];
  if query.starts_with(' ') {
    return None;
  }
  Some((slash, query))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn catalog_has_exact_22_items_with_stable_ids() {
    assert_eq!(SLASH_ITEMS.len(), 22);
    assert_eq!(SLASH_ITEMS[0].id, "slash-ai");
    assert_eq!(SLASH_ITEMS[14].id, "slash-code");
    assert_eq!(SLASH_ITEMS[21].id, "slash-image");
    assert_eq!(filter_slash_items("", true).len(), 22);
    assert_eq!(filter_slash_items("", false).len(), 21);
  }

  #[test]
  fn filter_matches_chinese_english_kind_alert_and_heading_shorthand() {
    assert!(filter_slash_items("代码", true).iter().any(|i| i.id == "slash-code"));
    assert!(filter_slash_items("code_block", true).iter().any(|i| i.id == "slash-code"));
    assert!(filter_slash_items("warning", true).iter().any(|i| i.id == "slash-alert-warning"));
    assert!(filter_slash_items("提示", true).iter().any(|i| i.id == "slash-alert-tip"));
    assert!(filter_slash_items("h2", true).iter().any(|i| i.id == "slash-h2"));
    assert!(filter_slash_items("mind", true).iter().any(|i| i.id == "slash-mindmap"));
    assert!(filter_slash_items("ai", false).is_empty());
    assert!(filter_slash_items("ai", true).iter().any(|i| i.id == "slash-ai"));
    assert!(filter_slash_items("zzz_no_match", true).is_empty());
  }

  #[test]
  fn extract_slash_query_handles_start_midline_and_rejections() {
    assert_eq!(extract_slash_query("/"), Some((0, "")));
    assert_eq!(extract_slash_query("/code"), Some((0, "code")));
    assert_eq!(extract_slash_query("Hello /tip"), Some((6, "tip")));
    assert_eq!(extract_slash_query("Line1\n/h1"), Some((6, "h1")));
    assert_eq!(extract_slash_query("/ "), None);
    assert_eq!(extract_slash_query("https://mica.dev"), None);
    assert_eq!(extract_slash_query("foo/bar"), None);
  }
}
