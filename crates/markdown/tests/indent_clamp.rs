//! P1-4: a `data.indent` written past the editor's 0..=8 cap (by an MCP/REST
//! client or an import) must still export at <=8 levels, matching what the Dart
//! editor renders — same data, same output across both engines.
use mica_markdown::{export_markdown, Block, DocumentSnapshotPayload};
use serde_json::json;

fn list_item(id: &str, text: &str, indent: u64) -> Block {
    Block {
        id: id.into(),
        kind: "bulleted_list".into(),
        text: text.into(),
        data: json!({ "indent": indent }),
        children: vec![],
    }
}

#[test]
fn list_indent_is_capped_at_8_like_the_editor() {
    let snapshot = DocumentSnapshotPayload {
        schema_version: 1,
        root_block_id: "root".to_string(),
        blocks: vec![
            Block {
                id: "root".into(),
                kind: "page".into(),
                text: String::new(),
                data: json!(null),
                children: vec!["a".into(), "b".into()],
            },
            list_item("a", "top", 0),
            list_item("b", "deep", 30),
        ],
    };
    let md = export_markdown(&snapshot).expect("export");
    eprintln!("EXPORT:\n{md}");
    let deep = md.lines().find(|l| l.contains("deep")).expect("deep line");
    let leading = deep.len() - deep.trim_start().len();
    // 8 levels * 4 spaces = 32. Without the cap, indent 30 → 120 spaces.
    assert!(leading <= 32, "indent must cap at 8 levels (<=32 spaces), got {leading}: {deep:?}");
    assert!(leading >= 4, "still indented, not flattened: {deep:?}");
}
