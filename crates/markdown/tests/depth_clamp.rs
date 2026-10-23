//! P1-6(10-19 #6a):quote / li 深度在渲染与导出前必须 clamp 到编辑器同一上限
//! (`MAX_NESTING_DEPTH` = 8,Dart `kMaxNestingDepth`)。
//!
//! 两条进程级崩溃路径:
//! ① 一行几万个 `>` → `data.quote = N` → `render_quote_group` 递归 N 层 → 栈溢出
//!   abort(分享页、HTML 导出);
//! ② REST/MCP 直接写 `data.li = 1e9` / `data.quote = 1e9`(`validate_block` 不看
//!   data)→ `export_md` 的 `" ".repeat(level * 4 + 4)` 一次要几 GB → 内存耗尽 abort,
//!   HTML 侧 `render_html_list` 递归 1e9 层。
//!
//! 每条都放进 512 KiB 栈的子线程跑:修复前是**整个测试进程** SIGSEGV/abort,
//! 修复后渲染深度被钉在 8,几 KB 栈就够。
use mica_markdown::{
  Block, DocumentSnapshotPayload, export_html, export_markdown, import_markdown,
};
use serde_json::{Value, json};

const SMALL_STACK: usize = 512 * 1024;

fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
  std::thread::Builder::new()
    .stack_size(SMALL_STACK)
    .spawn(f)
    .expect("spawn")
    .join()
    .expect("render must not panic")
}

fn doc(blocks: Vec<(&str, &str, &str, Value)>) -> DocumentSnapshotPayload {
  let mut all = vec![Block {
    id: "root".into(),
    kind: "page".into(),
    text: String::new(),
    data: json!(null),
    children: blocks.iter().map(|(id, ..)| id.to_string()).collect(),
  }];
  for (id, kind, text, data) in blocks {
    all.push(Block {
      id: id.into(),
      kind: kind.into(),
      text: text.into(),
      data,
      children: vec![],
    });
  }
  DocumentSnapshotPayload {
    schema_version: 1,
    root_block_id: "root".into(),
    blocks: all,
  }
}

/// 路径①:一行 10 万个 `>`。解析是迭代的(不爆),渲染只开 8 层 `<blockquote>`。
#[test]
fn a_line_of_100k_quote_markers_renders_at_most_8_levels() {
  let (html, md) = on_small_stack(|| {
    let src = format!("{}deep", ">".repeat(100_000));
    let snapshot = import_markdown(&src, "root");
    (
      export_html(&snapshot).expect("html"),
      export_markdown(&snapshot).expect("md"),
    )
  });
  assert!(html.contains("deep"), "content survives: {html}");
  assert_eq!(html.matches("<blockquote>").count(), 8, "{html}");
  let line = md.lines().find(|l| l.contains("deep")).expect("deep line");
  assert_eq!(
    line.matches('>').count(),
    8,
    "export caps the `>` prefix too: {line:?}"
  );
}

/// 路径②:非编辑器客户端写进来的 1e9 级深度。
#[test]
fn huge_li_and_quote_written_by_api_clients_are_capped() {
  let (html, md) = on_small_stack(|| {
    let snapshot = doc(vec![
      ("a", "bulleted_list", "item", json!({})),
      (
        "b",
        "paragraph",
        "li-child",
        json!({ "li": 1_000_000_000u64 }),
      ),
      ("c", "paragraph", "sep", json!({})),
      ("d", "quote", "q-deep", json!({ "quote": 1_000_000_000u64 })),
      ("e", "paragraph", "sep2", json!({})),
      (
        "f",
        "bulleted_list",
        "q-item",
        json!({ "quote": 1_000_000_000u64, "indent": 1_000_000_000u64 }),
      ),
    ]);
    (
      export_html(&snapshot).expect("html"),
      export_markdown(&snapshot).expect("md"),
    )
  });
  // `li-child` 不查 HTML:比它的 item 深的「孤儿」`data.li` 在 HTML 里本来就
  // 不出内容(与深度上限无关,li=1 挂在 0 级 item 下也一样),这里只钉崩溃。
  for needle in ["q-deep", "q-item"] {
    assert!(html.contains(needle), "{needle} rendered: {html}");
  }
  for needle in ["li-child", "q-deep", "q-item"] {
    let line = md.lines().find(|l| l.contains(needle)).expect(needle);
    let leading = line.len() - line.trim_start_matches([' ', '>']).len();
    // 8 层 li = 36 空格;8 层 quote = `> `×8 = 16;两者都远小于 64。
    assert!(
      leading <= 64,
      "{needle}: prefix must be capped, got {leading}: {line:?}"
    );
  }
  assert!(html.matches("<blockquote>").count() <= 16, "{html}");
}

/// 路径② 的 Markdown 导出单独钉住:不经过 HTML 递归,直接是 `repeat` 的分配。
#[test]
fn huge_li_does_not_allocate_gigabytes_on_markdown_export() {
  let started = std::time::Instant::now();
  let md = on_small_stack(|| {
    let snapshot = doc(vec![
      ("a", "bulleted_list", "item", json!({})),
      (
        "b",
        "paragraph",
        "li-child",
        json!({ "li": 1_000_000_000u64 }),
      ),
    ]);
    export_markdown(&snapshot).expect("md")
  });
  assert!(
    md.len() < 1024,
    "export must stay small, got {} bytes",
    md.len()
  );
  assert!(
    started.elapsed().as_secs() < 5,
    "took {:?}",
    started.elapsed()
  );
}

/// 上限内的深度原样保留(clamp 不是截平):7 层引用 import → export 字节稳定。
#[test]
fn depths_within_the_cap_round_trip_unchanged() {
  let src = "> > > > > > > seven\n";
  let md = export_markdown(&import_markdown(src, "root")).expect("md");
  assert_eq!(
    md.lines().find(|l| l.contains("seven")).unwrap(),
    src.trim_end()
  );
}
