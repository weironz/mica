use std::collections::HashSet;

use mica_interchange::notion::{looks_like_notion_export, strip_notion_id};
use mica_interchange::order::{natural_compare, order_page_paths};
use mica_interchange::zip::ZipFileEntry;
use mica_interchange::{build_zip, normalize_entries, plan_import, read_zip, resolve_ref};

fn fixture(name: &str) -> Vec<u8> {
  std::fs::read(format!(
    "{}/tests/fixtures/{name}",
    env!("CARGO_MANIFEST_DIR")
  ))
  .unwrap()
}

#[test]
fn reads_own_store_archives_round_trip() {
  let entries = vec![
    mica_interchange::ZipEntry {
      name: "Guide.md".into(),
      data: b"# Guide".to_vec(),
    },
    mica_interchange::ZipEntry {
      name: "assets/\u{56fe}.png".into(),
      data: vec![1, 2, 3],
    },
  ];
  let zip = build_zip(&entries);
  let back = read_zip(&zip).unwrap();
  assert_eq!(back.len(), 2);
  assert_eq!(back[0].name, "Guide.md");
  assert_eq!(back[0].bytes, b"# Guide");
  assert_eq!(back[1].name, "assets/\u{56fe}.png");
}

#[test]
fn reads_gbk_named_archive() {
  let entries = read_zip(&fixture("gbk.zip")).unwrap();
  let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
  assert!(names.contains(&"中文目录/页面.md"));
  assert!(names.contains(&"图片.png"));
  let md = entries.iter().find(|e| e.name.ends_with(".md")).unwrap();
  assert_eq!(String::from_utf8_lossy(&md.bytes), "GBK 内容");
}

#[test]
fn prefers_unicode_path_extra_field() {
  let entries = read_zip(&fixture("upath.zip")).unwrap();
  assert_eq!(entries[0].name, "测试.md");
  assert_eq!(entries[0].bytes, b"hi");
}

#[test]
fn reads_zip64_marked_archive() {
  let entries = read_zip(&fixture("zip64.zip")).unwrap();
  assert_eq!(entries[0].name, "Page.md");
  assert_eq!(String::from_utf8_lossy(&entries[0].bytes), "# 你好");
}

#[test]
fn reads_data_descriptor_archive() {
  let entries = read_zip(&fixture("dd.zip")).unwrap();
  assert_eq!(entries[0].name, "stream.md");
  assert_eq!(
    String::from_utf8_lossy(&entries[0].bytes),
    "streamed content"
  );
}

#[test]
fn normalize_peels_wrappers_and_drops_junk() {
  let e = |name: &str| ZipFileEntry {
    name: name.into(),
    bytes: Vec::new(),
  };
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
  let plan = plan_import(
    read_zip(&fixture("nested.zip")).unwrap(),
    false,
    mica_interchange::ImportMode::AsIs,
  )
  .unwrap();
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
  assert_eq!(
    strip_notion_id("My Page 1f2e3d4c5b6a7890abcdef1234567890"),
    "My Page"
  );
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
    [
      "Guide.md",
      "Notes.md",
      "x 0123456789abcdef0123456789abcdef.md"
    ]
    .into_iter()
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
      e(
        "apple 31f57556969b56ade626c2502854fc6d.md",
        "# apple\n\nfruit",
      ),
      e(
        "apple 0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f/iphone 31f57556969b81b5973cf30d40c5b6f1.md",
        "# iphone\n\nphone",
      ),
    ],
    true,
    mica_interchange::ImportMode::AsIs,
  )
  .unwrap();
  let titles: Vec<&str> = plan.pages.iter().map(|p| p.title.as_str()).collect();
  assert_eq!(titles, ["apple", "apple", "iphone"]);
  assert!(plan.pages[0].is_folder && !plan.pages[1].is_folder);
  assert_eq!(plan.pages[1].parent, Some(0));
  assert_eq!(plan.pages[2].parent, Some(0));
}
/// Issue #151: two Notion pages that differ ONLY by their trailing id, each
/// with its own body AND its own child directory, must stay two distinct
/// folders rather than merging into one. The single-page/differing-id pairing
/// (the test above) must keep working alongside this.
#[test]
fn notion_duplicate_titles_keep_separate_folder_identities() {
  let e = |name: &str, body: &str| ZipFileEntry {
    name: name.into(),
    bytes: body.as_bytes().to_vec(),
  };
  let plan = plan_import(
    vec![
      e("Doc-11111111111111111111111111111111.md", "# Doc\n\nfirst"),
      e("Doc-22222222222222222222222222222222.md", "# Doc\n\nsecond"),
      e(
        "Doc-11111111111111111111111111111111/ChildA.md",
        "# ChildA\n\na",
      ),
      e(
        "Doc-22222222222222222222222222222222/ChildB.md",
        "# ChildB\n\nb",
      ),
    ],
    true,
    mica_interchange::ImportMode::AsIs,
  )
  .unwrap();
  let folders: Vec<usize> = plan
    .pages
    .iter()
    .enumerate()
    .filter(|(_, p)| p.is_folder)
    .map(|(i, _)| i)
    .collect();
  assert_eq!(
    folders.len(),
    2,
    "expected two distinct Doc folders: {:#?}",
    plan
      .pages
      .iter()
      .map(|p| (&p.title, p.parent))
      .collect::<Vec<_>>()
  );
  assert_eq!(&plan.pages[folders[0]].title, "Doc");
  assert_eq!(&plan.pages[folders[1]].title, "Doc");

  let leaf = |body: &str| {
    plan
      .pages
      .iter()
      .position(|p| !p.is_folder && p.markdown.trim() == body)
      .unwrap_or_else(|| panic!("missing leaf {body:?}"))
  };
  let first = leaf("first");
  let second = leaf("second");
  assert!(folders.contains(&plan.pages[first].parent.unwrap()));
  assert!(folders.contains(&plan.pages[second].parent.unwrap()));
  assert_ne!(
    plan.pages[first].parent, plan.pages[second].parent,
    "two same-titled pages must not share one folder"
  );
  let child_a = plan.pages.iter().position(|p| p.title == "ChildA").unwrap();
  let child_b = plan.pages.iter().position(|p| p.title == "ChildB").unwrap();
  assert_eq!(plan.pages[child_a].parent, plan.pages[first].parent);
  assert_eq!(plan.pages[child_b].parent, plan.pages[second].parent);
}
/// Review follow-up to #151: document STEMS are claimants of a cleaned
/// directory name too. When only ONE of two same-titled pages owns a child
/// directory, the other page's body must not be folded into the first page's
/// folder — the body-less page stays a separate top-level leaf.
#[test]
fn notion_same_title_single_children_dir_keeps_pages_apart() {
  let a_id = "9f009f009f009f009f009f009f009f00";
  let b_id = "a1b2a1b2a1b2a1b2a1b2a1b2a1b2a1b2";
  let e = |name: &str, body: &str| ZipFileEntry {
    name: name.into(),
    bytes: body.as_bytes().to_vec(),
  };
  let plan = plan_import(
    vec![
      e(&format!("Doc {a_id}.md"), "A body"),
      e(&format!("Doc {a_id}/Sub {a_id}.md"), "sub"),
      e(&format!("Doc {b_id}.md"), "B body"),
    ],
    true,
    mica_interchange::ImportMode::AsIs,
  )
  .unwrap();
  let folder = plan
    .pages
    .iter()
    .position(|p| p.is_folder)
    .expect("one folder for the page with children");
  assert_eq!(plan.pages[folder].title, "Doc");
  let leaf = |body: &str| {
    plan
      .pages
      .iter()
      .position(|p| !p.is_folder && p.markdown.trim() == body)
      .unwrap_or_else(|| {
        panic!(
          "missing leaf {body:?}: {:#?}",
          plan
            .pages
            .iter()
            .map(|p| (&p.title, p.parent, p.markdown.trim()))
            .collect::<Vec<_>>()
        )
      })
  };
  let a = leaf("A body");
  let b = leaf("B body");
  // A owns the child directory → its body becomes a leaf inside the folder.
  assert_eq!(plan.pages[a].parent, Some(folder));
  // B has NO children of its own → it must stay a separate top-level page,
  // not be absorbed into A's folder.
  assert_eq!(plan.pages[b].parent, None, "B was merged into A's folder");
}

#[test]
fn ordering_rules() {
  let manifest = r#"{"version":1,"pages":[{"path":"Z.md"},{"path":"Z/I.md"},{"path":"A.md"}]}"#;
  assert_eq!(
    order_page_paths(
      vec![
        "A.md".into(),
        "Z/I.md".into(),
        "Z.md".into(),
        "New.md".into()
      ],
      Some(manifest)
    ),
    ["Z.md", "Z/I.md", "A.md", "New.md"]
  );
  assert_eq!(
    order_page_paths(
      vec![
        "b/10.md".into(),
        "b/2.md".into(),
        "b.md".into(),
        "10 篇.md".into(),
        "2 篇.md".into()
      ],
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
    resolve_ref(
      "Guide/Setup/Linux.md",
      "assets/%E5%9B%BE%E7%89%87.png",
      &paths
    )
    .as_deref(),
    Some("assets/图片.png")
  );
  assert_eq!(resolve_ref("a.md", "https://x.com/i.png", &paths), None);

  // Typora writes the display size after the path: `![](x.png =100%)`. That is
  // not CommonMark, so the whole thing arrives as one "url" — and used to miss
  // both the path match and the basename fallback, dropping the image and
  // leaving a dead relative link in the page. Real archives carried 8 of these.
  for size in ["=100%", "=629", "=800x600"] {
    assert_eq!(
      resolve_ref(
        "Guide/Setup/Linux.md",
        &format!("../../assets/图片.png {size}"),
        &paths
      )
      .as_deref(),
      Some("assets/图片.png"),
      "size suffix {size} must not defeat resolution"
    );
  }

  // …but a filename is allowed to contain " =" itself. Stripping blindly at the
  // last space would break every "Screen Shot 2024.png" in the world, and a
  // wrong strip turns a RESOLVABLE reference into an unresolvable one — worse
  // than the bug being fixed.
  let odd: HashSet<String> = [
    "assets/budget =2024.png".to_string(),
    "assets/Screen Shot.png".to_string(),
  ]
  .into();
  assert_eq!(
    resolve_ref("a.md", "assets/budget =2024.png", &odd).as_deref(),
    Some("assets/budget =2024.png"),
    "`=2024` is not a size token — the name keeps it"
  );
  assert_eq!(
    resolve_ref("a.md", "assets/Screen Shot.png", &odd).as_deref(),
    Some("assets/Screen Shot.png"),
    "a space in a filename is not a size suffix"
  );
  assert_eq!(
    resolve_ref("a.md", "data:image/png;base64,xx", &paths),
    None
  );
  assert_eq!(resolve_ref("a.md", "nope.png", &paths), None);
}

#[test]
fn plan_full_archive_with_manifest_links_and_assets() {
  let plan = plan_import(
    read_zip(&fixture("plan.zip")).unwrap(),
    false,
    mica_interchange::ImportMode::AsIs,
  )
  .unwrap();
  assert!(!plan.notion);
  let titles: Vec<&str> = plan.pages.iter().map(|p| p.title.as_str()).collect();
  // Manifest order, not alphabetical. `Zeta.md` owns `Zeta/` → folder + leaf.
  assert_eq!(titles, ["Zeta", "Zeta", "Sub", "Alpha"]);
  assert!(plan.pages[0].is_folder && !plan.pages[1].is_folder);
  assert_eq!(plan.pages[2].parent, Some(0)); // Zeta/Sub.md under the Zeta FOLDER
  assert_eq!(plan.pages[3].parent, None);
  // Body verbatim: this fixture's manifest predates `generator: "mica"`, so it is
  // not known to be ours and a leading heading is the author's, not ours to eat.
  // (Our current exports and Notion's DO weld the title on as an H1 — those are
  // stripped at import; see `mica_markdown::strip_page_title`'s call site.)
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
/// Issue #93: dot-prefixed page/folder names were exported as path segments
/// that the importer's own hidden-file filter then discarded — `.hidden`
/// pages lost their whole body, `.private/Child` lost the `Child`, and a
/// folder named `..` produced a parent-traversal zip entry. The export must
/// now emit only import-safe segments; display names ride on the manifest.
#[test]
fn dot_prefixed_pages_survive_round_trip() {
  use std::collections::HashMap;

  use mica_interchange::{ImportMode, TreeNode, build_markdown_tree_zip};

  let node = |id: &str, name: &str, parent: Option<&str>, folder: bool| TreeNode {
    id: id.into(),
    name: name.into(),
    parent_id: parent.map(str::to_string),
    position: id.into(),
    object_id: format!("doc-{id}"),
    object_type: if folder {
      "folder".into()
    } else {
      "document".into()
    },
    icon: None,
  };
  let nodes = vec![
    node("a", "Visible", None, false),
    // Sibling whose stripped spelling collides with `.hidden`'s: proves the
    // encoded names dedup instead of overwriting.
    node("g", "hidden", None, false),
    node("b", ".hidden", None, false),
    node("c", ".private", None, true),
    node("d", "Child", Some("c"), false),
    node("e", "..", None, true),
    node("f", "Note", Some("e"), false),
  ];
  let payloads: HashMap<_, _> = nodes
    .iter()
    .filter(|n| n.object_type == "document")
    .map(|n| {
      (
        n.object_id.clone(),
        mica_markdown::import_markdown("Important text", &format!("root-{}", n.id)),
      )
    })
    .collect();
  let entries = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
  for e in &entries {
    assert!(
      !e.name.split('/').any(|seg| seg == "." || seg == ".."),
      "parent-traversal entry: {}",
      e.name
    );
    assert!(
      !e.name.split('/').any(|seg| seg.starts_with('.')),
      "hidden (import-junk) entry: {}",
      e.name
    );
  }
  let plan = plan_import(
    read_zip(&build_zip(&entries)).unwrap(),
    false,
    ImportMode::AsIs,
  )
  .unwrap();
  let page = |title: &str| {
    plan
      .pages
      .iter()
      .find(|p| p.title == title)
      .unwrap_or_else(|| {
        panic!(
          "missing page {title:?} in {:#?}",
          plan.pages.iter().map(|p| &p.title).collect::<Vec<_>>()
        )
      })
  };
  assert_eq!(page("Visible").markdown.trim(), "Important text");
  assert_eq!(page(".hidden").markdown.trim(), "Important text");
  assert_eq!(page("hidden").markdown.trim(), "Important text");
  assert!(page(".private").is_folder);
  let child = page("Child");
  assert_eq!(child.markdown.trim(), "Important text");
  assert_eq!(
    child.parent,
    plan.pages.iter().position(|p| p.title == ".private")
  );
  assert!(page("..").is_folder);
  let note = page("Note");
  assert_eq!(note.markdown.trim(), "Important text");
  assert_eq!(note.parent, plan.pages.iter().position(|p| p.title == ".."));
}
/// Issue #143: two inner archives each carrying the SAME relative paths
/// (`assets/logo.png`, `A.md`, `B.md`) with DIFFERENT bytes must stay apart —
/// today `plan_import` flattens them and the later archive silently overwrites
/// the earlier one's pages and asset. Each inner archive gets its own namespace.
#[test]
fn nested_part_archives_keep_same_path_namespaces_apart() {
  let inner = |logo: &[u8], a: &str, b: &str| {
    build_zip(&[
      mica_interchange::ZipEntry {
        name: "assets/logo.png".into(),
        data: logo.to_vec(),
      },
      mica_interchange::ZipEntry {
        name: "A.md".into(),
        data: a.as_bytes().to_vec(),
      },
      mica_interchange::ZipEntry {
        name: "B.md".into(),
        data: b.as_bytes().to_vec(),
      },
    ])
  };
  let part1 = inner(b"PART_ONE", "# P1A\n\n[logo](assets/logo.png)", "# P1B");
  let part2 = inner(b"PART_TWO", "# P2A\n\n[logo](assets/logo.png)", "# P2B");
  let outer = build_zip(&[
    mica_interchange::ZipEntry {
      name: "Part1.zip".into(),
      data: part1,
    },
    mica_interchange::ZipEntry {
      name: "Part2.zip".into(),
      data: part2,
    },
  ]);

  let plan = plan_import(
    read_zip(&outer).unwrap(),
    false,
    mica_interchange::ImportMode::AsIs,
  )
  .unwrap();
  // Both assets survive, each under its inner archive's namespace.
  assert_eq!(
    plan.files.get("Part1/assets/logo.png").map(Vec::as_slice),
    Some(&b"PART_ONE"[..])
  );
  assert_eq!(
    plan.files.get("Part2/assets/logo.png").map(Vec::as_slice),
    Some(&b"PART_TWO"[..])
  );
  // Both pages survive too (the later archive must not overwrite the earlier).
  let bodies: Vec<&str> = plan
    .pages
    .iter()
    .filter(|p| !p.is_folder)
    .map(|p| p.markdown.trim())
    .collect();
  assert!(
    bodies.contains(&"# P1A\n\n[logo](assets/logo.png)"),
    "Part1 page lost: {bodies:?}"
  );
  assert!(
    bodies.contains(&"# P2A\n\n[logo](assets/logo.png)"),
    "Part2 page lost: {bodies:?}"
  );
}
/// Issue #144 (export side): internal page links inside TABLE cells must be
/// relativised to `B.md` exactly like the ones in paragraphs — not left as
/// `mica://page/…` which is dead in any external Markdown viewer.
#[test]
fn table_cell_page_links_relativise_in_zip_export() {
  use std::collections::HashMap;

  use mica_interchange::{TreeNode, build_markdown_tree_zip};

  let node = |id: &str, name: &str| TreeNode {
    id: id.into(),
    name: name.into(),
    parent_id: None,
    position: id.into(),
    object_id: format!("doc-{id}"),
    object_type: "document".into(),
    icon: None,
  };
  let nodes = vec![node("a", "A"), node("b", "B")];
  let payloads: HashMap<_, _> = [(
    "doc-a".to_string(),
    mica_markdown::import_markdown("# A\n\n|H|\n|---|\n|[B](mica://page/b)|", "root-a"),
  )]
  .into();
  let entries = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
  let md = entries
    .iter()
    .find(|e| e.name == "A.md")
    .expect("A.md present");
  let body = String::from_utf8_lossy(&md.data);
  assert!(
    body.contains("[B](B.md)"),
    "table-cell link must be relativised, got:\n{body}"
  );
}

/// Issue #144 (import side): a reference definition (`[ref]: /target`) must
/// resolve inside a table cell exactly as it does in a paragraph — the cell
/// exports the resolved link, not a literal `[ref]`.
#[test]
fn table_cell_reference_definitions_resolve_like_paragraphs() {
  let payload = mica_markdown::import_markdown("[ref]: /target\n\n|H|\n|---|\n| [ref] |", "root");
  let out = mica_markdown::export_markdown(&payload).expect("exports");
  assert!(
    out.contains("| [ref](/target) |"),
    "cell reference must resolve to a link, got:\n{out}"
  );
}
/// Review finding 2: re-rooting a conflicted inner archive under its own stem
/// can itself collide with an outer path. The outer `A/note.md` and the inner
/// archive's `note.md` (re-rooted as `A/note.md`) must NOT overwrite each
/// other — the re-root prefix must be unique against every group's paths.
#[test]
fn conflict_reroot_prefix_avoids_existing_outer_paths() {
  let ze = |name: &str, data: &[u8]| mica_interchange::ZipEntry {
    name: name.into(),
    data: data.to_vec(),
  };
  let inner = build_zip(&[ze("index.md", b"# Inner"), ze("note.md", b"# InnerNote")]);
  let outer = build_zip(&[
    ze("index.md", b"# Outer"),
    ze("A/note.md", b"# OuterNote"),
    ze("A.zip", &inner),
  ]);
  let plan = plan_import(
    read_zip(&outer).unwrap(),
    false,
    mica_interchange::ImportMode::AsIs,
  )
  .unwrap();
  let bodies: Vec<&str> = plan.pages.iter().map(|p| p.markdown.trim()).collect();
  assert!(
    bodies.contains(&"# Outer"),
    "outer index.md lost: {bodies:?}"
  );
  assert!(
    bodies.contains(&"# Inner"),
    "inner index.md lost: {bodies:?}"
  );
  assert!(
    bodies.contains(&"# OuterNote"),
    "outer A/note.md lost: {bodies:?}"
  );
  assert!(
    bodies.contains(&"# InnerNote"),
    "inner note.md lost: {bodies:?}"
  );
}

/// Review finding 4: a page or folder whose name ENDS in a dot is forbidden
/// on Windows (and `Docs.` canonicalizes to `Docs`, colliding two siblings on
/// extraction). The exporter must fold trailing dots just like leading ones.
#[test]
fn trailing_dot_names_export_windows_safe_segments() {
  use std::collections::HashMap;

  use mica_interchange::{ImportMode, TreeNode, build_markdown_tree_zip};

  let node = |id: &str, name: &str, parent: Option<&str>, folder: bool| TreeNode {
    id: id.into(),
    name: name.into(),
    parent_id: parent.map(str::to_string),
    position: id.into(),
    object_id: format!("doc-{id}"),
    object_type: if folder {
      "folder".into()
    } else {
      "document".into()
    },
    icon: None,
  };
  let nodes = vec![
    node("a", "Docs", None, true),
    node("b", "One", Some("a"), false),
    node("c", "Docs.", None, true),
    node("d", "Two", Some("c"), false),
  ];
  let payloads: HashMap<_, _> = nodes
    .iter()
    .filter(|n| n.object_type == "document")
    .map(|n| {
      (
        n.object_id.clone(),
        mica_markdown::import_markdown("text", &format!("root-{}", n.id)),
      )
    })
    .collect();
  let entries = build_markdown_tree_zip(&nodes, None, &payloads, &HashMap::new());
  let mut segments = std::collections::HashSet::new();
  for e in &entries {
    for seg in e.name.split('/') {
      assert!(
        !seg.ends_with('.') || seg == "manifest.json",
        "trailing-dot segment in {}",
        e.name
      );
      segments.insert(seg.to_lowercase());
    }
  }
  let plan = plan_import(
    read_zip(&build_zip(&entries)).unwrap(),
    false,
    ImportMode::AsIs,
  )
  .unwrap();
  let folder_titles: Vec<&str> = plan
    .pages
    .iter()
    .filter(|p| p.is_folder)
    .map(|p| p.title.as_str())
    .collect();
  assert_eq!(folder_titles, ["Docs", "Docs."]);
  assert!(plan.pages.iter().any(|p| p.title == "One"));
  assert!(plan.pages.iter().any(|p| p.title == "Two"));
}

/// Review finding 5: a NESTED inner archive whose entry fails CRC must not be
/// silently swallowed as a dropped unreadable inner archive — the corruption
/// is detectable, so import must fail, not complete with only the good pages.
#[test]
fn corrupt_nested_archive_is_not_silently_dropped() {
  let mut inner = build_zip(&[mica_interchange::ZipEntry {
    name: "B.md".into(),
    data: b"inner body".to_vec(),
  }]);
  let at = inner
    .windows(10)
    .position(|w| w == b"inner body")
    .expect("payload present");
  inner[at] = b'X'; // single-byte corruption; the recorded CRC stays
  let outer = build_zip(&[
    mica_interchange::ZipEntry {
      name: "A.md".into(),
      data: b"# A\n\nouter body".to_vec(),
    },
    mica_interchange::ZipEntry {
      name: "B.zip".into(),
      data: inner,
    },
  ]);
  // Today the corrupt inner archive is swallowed as "unreadable inner zip,
  // dropped" — the import completes with only A. It must instead ERROR.
  let err = plan_import(
    read_zip(&outer).unwrap(),
    false,
    mica_interchange::ImportMode::AsIs,
  )
  .expect_err("corrupt nested archive must fail the import");
  assert!(err.to_string().contains("B.md"), "{err}");
}
