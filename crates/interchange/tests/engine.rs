use std::collections::HashSet;

use mica_interchange::notion::{looks_like_notion_export, strip_notion_id};
use mica_interchange::order::{natural_compare, order_page_paths};
use mica_interchange::zip::ZipFileEntry;
use mica_interchange::{build_zip, normalize_entries, plan_import, read_zip, resolve_ref};

fn fixture(name: &str) -> Vec<u8> {
  std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn reads_own_store_archives_round_trip() {
  let entries = vec![
    mica_interchange::ZipEntry { name: "Guide.md".into(), data: b"# Guide".to_vec() },
    mica_interchange::ZipEntry { name: "assets/\u{56fe}.png".into(), data: vec![1, 2, 3] },
  ];
  let zip = build_zip(&entries);
  let back = read_zip(&zip);
  assert_eq!(back.len(), 2);
  assert_eq!(back[0].name, "Guide.md");
  assert_eq!(back[0].bytes, b"# Guide");
  assert_eq!(back[1].name, "assets/\u{56fe}.png");
}

#[test]
fn reads_gbk_named_archive() {
  let entries = read_zip(&fixture("gbk.zip"));
  let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
  assert!(names.contains(&"中文目录/页面.md"));
  assert!(names.contains(&"图片.png"));
  let md = entries.iter().find(|e| e.name.ends_with(".md")).unwrap();
  assert_eq!(String::from_utf8_lossy(&md.bytes), "GBK 内容");
}

#[test]
fn prefers_unicode_path_extra_field() {
  let entries = read_zip(&fixture("upath.zip"));
  assert_eq!(entries[0].name, "测试.md");
  assert_eq!(entries[0].bytes, b"hi");
}

#[test]
fn reads_zip64_marked_archive() {
  let entries = read_zip(&fixture("zip64.zip"));
  assert_eq!(entries[0].name, "Page.md");
  assert_eq!(String::from_utf8_lossy(&entries[0].bytes), "# 你好");
}

#[test]
fn reads_data_descriptor_archive() {
  let entries = read_zip(&fixture("dd.zip"));
  assert_eq!(entries[0].name, "stream.md");
  assert_eq!(String::from_utf8_lossy(&entries[0].bytes), "streamed content");
}

#[test]
fn normalize_peels_wrappers_and_drops_junk() {
  let e = |name: &str| ZipFileEntry { name: name.into(), bytes: Vec::new() };
  let out = normalize_entries(vec![
    e("__MACOSX/notes/._a.md"),
    e("vault/.obsidian/app.json"),
    e("vault/.trash/deleted.md"),
    e("vault/Thumbs.db"),
    e("vault/note.md"),
    e("vault/pics/b.png"),
  ]);
  let names: Vec<&str> = out.iter().map(|x| x.name.as_str()).collect();
  assert_eq!(names, ["note.md", "pics/b.png"]);

  // A root file blocks peeling (Mica exports keep manifest.json at root).
  let out = normalize_entries(vec![e("manifest.json"), e("Guide/Setup.md")]);
  assert_eq!(out.len(), 2);
  assert_eq!(out[0].name, "manifest.json");
}

#[test]
fn nested_part_zip_expands_through_plan() {
  let plan = plan_import(read_zip(&fixture("nested.zip")), false, mica_interchange::ImportMode::AsIs);
  assert!(plan.notion); // names carry 32-hex ids → auto-detected
  let titles: Vec<&str> = plan.pages.iter().map(|p| p.title.as_str()).collect();
  // `Guide.md` has a `Guide/` of children → folder "Guide" + leaf "Guide" holding
  // the body (Mica pages are leaves; see `page_with_subpages_splits_into_folder_plus_leaf`).
  assert_eq!(titles, ["Guide", "Guide", "Sub"]);
  assert!(plan.pages[0].is_folder && !plan.pages[1].is_folder);
  assert_eq!(plan.pages[1].parent, Some(0));
  assert_eq!(plan.pages[1].markdown.trim(), "inner");
  assert_eq!(plan.pages[2].parent, Some(0)); // Sub hangs off the FOLDER
}

#[test]
fn notion_helpers() {
  assert_eq!(strip_notion_id("My Page 1f2e3d4c5b6a7890abcdef1234567890"), "My Page");
  assert_eq!(
    strip_notion_id("Export-1f2e3d4c-5b6a-7890-abcd-ef1234567890"),
    "Export"
  );
  assert_eq!(strip_notion_id("2024 总结"), "2024 总结");
  assert_eq!(strip_notion_id("deadbeef"), "deadbeef");

  assert!(looks_like_notion_export(
    ["apple 31f57556969b56ade626c2502854fc6d.md", "notes.md"].into_iter()
  ));
  assert!(!looks_like_notion_export(
    ["Guide.md", "Notes.md", "x 0123456789abcdef0123456789abcdef.md"].into_iter()
  ));
}

/// The Notion shape end to end: a folder and its page carry DIFFERENT id
/// suffixes for the same logical page. They must still pair up into one
/// folder-plus-leaf, not import as two unrelated nodes.
#[test]
fn notion_folder_and_page_pair_up_across_differing_ids() {
  let e = |name: &str, body: &str| ZipFileEntry {
    name: name.into(),
    bytes: body.as_bytes().to_vec(),
  };
  let plan = plan_import(
    vec![
      e("apple 31f57556969b56ade626c2502854fc6d.md", "# apple\n\nfruit"),
      e("apple 0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f/iphone 31f57556969b81b5973cf30d40c5b6f1.md",
        "# iphone\n\nphone"),
    ],
    true,
    mica_interchange::ImportMode::AsIs,
  );
  let titles: Vec<&str> = plan.pages.iter().map(|p| p.title.as_str()).collect();
  assert_eq!(titles, ["apple", "apple", "iphone"]);
  assert!(plan.pages[0].is_folder && !plan.pages[1].is_folder);
  assert_eq!(plan.pages[1].parent, Some(0));
  assert_eq!(plan.pages[2].parent, Some(0));
}

#[test]
fn ordering_rules() {
  let manifest = r#"{"version":1,"pages":[{"path":"Z.md"},{"path":"Z/I.md"},{"path":"A.md"}]}"#;
  assert_eq!(
    order_page_paths(
      vec!["A.md".into(), "Z/I.md".into(), "Z.md".into(), "New.md".into()],
      Some(manifest)
    ),
    ["Z.md", "Z/I.md", "A.md", "New.md"]
  );
  assert_eq!(
    order_page_paths(
      vec!["b/10.md".into(), "b/2.md".into(), "b.md".into(), "10 篇.md".into(), "2 篇.md".into()],
      None
    ),
    ["2 篇.md", "10 篇.md", "b.md", "b/2.md", "b/10.md"]
  );
  assert!(natural_compare("第2章", "第10章").is_lt());
  // Malformed manifest falls back gracefully.
  assert_eq!(
    order_page_paths(vec!["b.md".into(), "a.md".into()], Some("{not json")),
    ["a.md", "b.md"]
  );
}

#[test]
fn resolve_ref_rules() {
  let paths: HashSet<String> = [
    "assets/图片.png".to_string(),
    "Guide/pics/a.png".to_string(),
    "Guide/Setup/shot.png".to_string(),
  ]
  .into();
  assert_eq!(
    resolve_ref("Guide/Setup.md", "pics/a.png", &paths).as_deref(),
    Some("Guide/pics/a.png")
  );
  assert_eq!(
    resolve_ref("Guide/Setup/Linux.md", "../../assets/图片.png", &paths).as_deref(),
    Some("assets/图片.png")
  );
  // Root-relative fallback + percent decoding.
  assert_eq!(
    resolve_ref("Guide/Setup/Linux.md", "assets/%E5%9B%BE%E7%89%87.png", &paths).as_deref(),
    Some("assets/图片.png")
  );
  assert_eq!(resolve_ref("a.md", "https://x.com/i.png", &paths), None);
  assert_eq!(resolve_ref("a.md", "data:image/png;base64,xx", &paths), None);
  assert_eq!(resolve_ref("a.md", "nope.png", &paths), None);
}

#[test]
fn plan_full_archive_with_manifest_links_and_assets() {
  let plan = plan_import(read_zip(&fixture("plan.zip")), false, mica_interchange::ImportMode::AsIs);
  assert!(!plan.notion);
  let titles: Vec<&str> = plan.pages.iter().map(|p| p.title.as_str()).collect();
  // Manifest order, not alphabetical. `Zeta.md` owns `Zeta/` → folder + leaf.
  assert_eq!(titles, ["Zeta", "Zeta", "Sub", "Alpha"]);
  assert!(plan.pages[0].is_folder && !plan.pages[1].is_folder);
  assert_eq!(plan.pages[2].parent, Some(0)); // Zeta/Sub.md under the Zeta FOLDER
  assert_eq!(plan.pages[3].parent, None);
  // Body verbatim: our own archives never weld the name into the text, so there
  // is nothing to strip and a leading heading is the author's, not ours to eat.
  // (Notion archives DO repeat the title as an H1 — that one case is stripped at
  // import; see `strip_leading_h1`'s call site.)
  assert!(plan.pages[1].markdown.contains("see [Sub](Zeta/Sub.md)"));
  assert!(plan.pages[1].markdown.contains("# Zeta"));
  // Asset present and resolvable from the page that references it.
  assert!(plan.files.contains_key("assets/p.png"));
  let file_paths: HashSet<String> = plan.files.keys().cloned().collect();
  assert_eq!(
    resolve_ref("Zeta.md", "assets/p.png", &file_paths).as_deref(),
    Some("assets/p.png")
  );
  // Link target resolves to the planned page.
  let target = resolve_ref("Zeta/Sub.md", "../Zeta.md", &plan.md_paths).unwrap();
  assert_eq!(plan.page_by_path[&target], 1); // the leaf that carries Zeta's body
}
