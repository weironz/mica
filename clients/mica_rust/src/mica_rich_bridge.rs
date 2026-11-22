//! Isomorphic flat-block ProseMirror [`Schema`] and [`MicaDocumentBridge`]
//! connecting `mica_core::MicaDoc` (yrs CRDT, sole storage & Undo/Redo truth)
//! with `nsh_base::rich_editor::view::EditorSession`.
//!
//! Architecture invariants:
//! - Flat top-level block model: every top-level block carries a stable UUID
//!   `id` attribute matching its `mica_core::Block::id` in `MicaDoc`.
//! - Single history stack: `rich_editor::extensions::history` is NEVER
//!   installed. `Mod-z` / `Mod-y` / `Shift-Mod-z` drive `MicaDoc::undo()` /
//!   `MicaDoc::redo()` directly.
//! - Incremental CRDT projection: local transactions diff top-level blocks and
//!   apply only changed/inserted/moved/deleted block ops inside a single
//!   `MicaDoc` history batch, then emit Markdown + incremental yrs update bytes.

use std::{
  cell::RefCell,
  collections::{BTreeMap, HashMap, HashSet},
  mem::ManuallyDrop,
  rc::{Rc, Weak},
  sync::Arc,
  thread::ThreadId,
};

use mica_core::{Block, MicaDoc, marks as core_marks};
use mica_md::{
  Document, HtmlOptions, InlineText, Mark as MdMark, ParseOptions, SCHEMA_VERSION,
  SerializeOptions, decode_cell, encode_cell, parse, render_html, serialize,
};
use nsh_base::rich_editor::{
  commands::{
    BoxCommand, Dispatch, SplitAs, command, is_mark_active, split_block_as, tiptap::FOCUS_META,
    toggle_mark,
  },
  extensions::{
    core_keymap,
    input_rules::{input_rules, undo_input_rule},
    keymap::keymap,
    kit, starter,
    trailing_node::{SKIP_TRAILING_NODE, TrailingNodeOptions, trailing_node},
  },
  model::{
    AttrSpec, AttrType, Attrs, DomOutputSpec, Fragment, Mark as PmMark, MarkSpec, Node, NodeSpec,
    ParseRule, Schema, SchemaSpec, Whitespace, utf16_len, utf16_to_byte,
  },
  state::{
    EditorState, EditorStateConfig, KeyEvent, Plugin, PluginKey, PluginSpec, Selection,
    Transaction, TransactionOrigin,
  },
  tables::{
    self, TableEditingOptions, commands as table_commands, fix_tables_plugin, nodes as table_nodes,
    table_editing,
  },
  view::{EditorSession, NodeViews},
};
use nsh_core::prelude::{Signal, signal};
use serde_json::{Map, Value, json};
use uuid::Uuid;

const MICA_SYNC_ORIGIN_META: &str = "micaSyncOrigin";
const DEFAULT_ROOT_ID: &str = "root";
const DEFAULT_STATUS_TEXT: &str = "上次编辑 刚刚";
const MAX_INDENT: u64 = 8;
const MARK_NAMES: [&str; 6] = ["bold", "italic", "strike", "code", "link", "underline"];

/// One heading entry in the live page outline (consumed by `right_sidebar.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineItem {
  pub block_index: usize,
  pub pos: usize,
  pub level: u64,
  pub text: String,
}

// ── Attr & Small Helpers ─────────────────────────────────────────────────────

fn attr_str<'a>(attrs: &'a Attrs, key: &str) -> &'a str {
  attrs.get(key).and_then(Value::as_str).unwrap_or_default()
}
fn attr_u64(attrs: &Attrs, key: &str, default: u64) -> u64 {
  attrs.get(key).and_then(Value::as_u64).unwrap_or(default)
}
fn attr_bool(attrs: &Attrs, key: &str) -> bool {
  attrs.get(key).and_then(Value::as_bool).unwrap_or(false)
}
fn new_block_id() -> String {
  Uuid::new_v4().to_string()
}
fn is_indentable_kind(name: &str) -> bool {
  matches!(name, "paragraph" | "heading" | "bulleted_list" | "numbered_list" | "todo" | "quote")
}
fn make_attrs(id: String, extra: &[(&str, Value)]) -> Attrs {
  let mut attrs = Attrs::new();
  attrs.insert("id".into(), Value::String(id));
  for (k, v) in extra {
    attrs.insert((*k).to_owned(), v.clone());
  }
  attrs
}
fn set_if_changed<T: Clone + PartialEq + 'static>(sig: Signal<T>, val: T) {
  if sig.get_untracked() != val {
    sig.set(val);
  }
}

// ── 1. Isomorphic Flat-Block Schema ──────────────────────────────────────────

fn attr_spec(default: Value, validate: AttrType, keep_on_split: bool) -> AttrSpec {
  let mut s = AttrSpec::default();
  s.default = Some(default);
  s.validate = Some(validate);
  s.keep_on_split = keep_on_split;
  s.rendered = false;
  s
}
fn id_attr() -> (String, AttrSpec) {
  ("id".into(), attr_spec(Value::String(String::new()), AttrType::String, false))
}
fn indent_attr() -> (String, AttrSpec) {
  ("indent".into(), attr_spec(Value::from(0), AttrType::Number, true))
}
fn str_attr(name: &str, keep_on_split: bool) -> (String, AttrSpec) {
  (name.into(), attr_spec(Value::String(String::new()), AttrType::String, keep_on_split))
}
fn flat_textblock(tag: &'static str, defining: bool, extra: Vec<(String, AttrSpec)>) -> NodeSpec {
  let mut s = NodeSpec::default();
  s.content = Some("inline*".into());
  s.group = Some("block".into());
  s.defining = defining;
  s.attrs = std::iter::once(id_attr()).chain(extra).chain(std::iter::once(indent_attr())).collect();
  s.to_dom = Some(DomOutputSpec::tag(tag).hole());
  s
}
fn atom_block(tag: &'static str, extra: Vec<(String, AttrSpec)>) -> NodeSpec {
  let mut s = NodeSpec::default();
  s.group = Some("block".into());
  s.atom = true;
  s.attrs = std::iter::once(id_attr()).chain(extra).collect();
  s.to_dom = Some(DomOutputSpec::tag(tag));
  s
}

/// Builds Mica's isomorphic flat-block ProseMirror [`Schema`].
pub fn mica_schema() -> Schema {
  let mut doc_spec = NodeSpec::default();
  doc_spec.content = Some("block+".into());

  // `paragraph` MUST come first in the `block` group so `doc`'s default block is `paragraph`.
  let mut paragraph = flat_textblock("p", false, vec![]);
  paragraph.parse_dom = vec![ParseRule::tag("p")];

  let mut heading = flat_textblock(
    "h1",
    true,
    vec![("level".into(), attr_spec(Value::from(1), AttrType::Number, true))],
  );
  heading.to_dom = Some(DomOutputSpec::render(|src| {
    let lvl = src.attr("level").and_then(Value::as_u64).unwrap_or(1).clamp(1, 6);
    DomOutputSpec::tag(format!("h{lvl}")).hole()
  }));
  heading.parse_dom = (1..=6_u64)
    .map(|lvl| {
      let mut a = Attrs::new();
      a.insert("level".into(), Value::from(lvl));
      ParseRule::tag(format!("h{lvl}")).with_attrs(a)
    })
    .collect();

  let mut todo = flat_textblock(
    "div",
    true,
    vec![("checked".into(), attr_spec(Value::Bool(false), AttrType::Boolean, false))],
  );
  todo.extra.insert("render".into(), Value::String("task".into()));

  let mut quote = flat_textblock("blockquote", true, vec![]);
  quote.parse_dom = vec![ParseRule::tag("blockquote")];

  let mut code_block = NodeSpec::default();
  code_block.content = Some("text*".into());
  code_block.marks = Some(String::new());
  code_block.group = Some("block".into());
  code_block.code = true;
  code_block.defining = true;
  code_block.attrs = vec![id_attr(), str_attr("language", true)];
  code_block.to_dom = Some(DomOutputSpec::render(|src| {
    let lang = src.attr("language").and_then(Value::as_str).unwrap_or_default();
    let inner = if lang.is_empty() {
      DomOutputSpec::tag("code")
    } else {
      DomOutputSpec::tag("code").attr("class", format!("language-{lang}"))
    };
    DomOutputSpec::tag("pre").child(inner.hole())
  }));
  let mut pre_rule = ParseRule::tag("pre");
  pre_rule.preserve_whitespace = Some(Whitespace::Full);
  code_block.parse_dom = vec![pre_rule];

  let mut divider = atom_block("hr", vec![]);
  divider.parse_dom = vec![ParseRule::tag("hr")];

  let mut image = atom_block("img", vec![str_attr("src", false), str_attr("alt", false)]);
  image.to_dom = Some(DomOutputSpec::tag("img").attr_of("src", "src").attr_of("alt", "alt"));
  image.parse_dom = vec![ParseRule::tag("img[src]").with_get_attrs(|input| {
    let el = input.element()?;
    let mut a = Attrs::new();
    a.insert("src".into(), Value::String(el.attr("src")?.to_owned()));
    a.insert("alt".into(), Value::String(el.attr("alt").unwrap_or_default().to_owned()));
    Some(a)
  })];

  // Standard ProseMirror tables (`nsh_base::rich_editor::tables`), plus `id` and `aligns`.
  let (table_name, mut table_spec) = table_nodes::table(&json!({}));
  table_spec.attrs.push(id_attr());
  table_spec.attrs.push(("aligns".into(), attr_spec(Value::Null, AttrType::Any, false)));

  let mut text_spec = NodeSpec::default();
  text_spec.group = Some("inline".into());

  let mut code_mark = MarkSpec::default();
  code_mark.code = true;
  code_mark.exitable = true;
  code_mark.to_dom = Some(DomOutputSpec::tag("code").hole());
  code_mark.parse_dom = vec![ParseRule::tag("code")];
  code_mark.extra.insert("code".into(), Value::Bool(true));

  let mut link_mark = MarkSpec::default();
  link_mark.attrs = vec![str_attr("href", true)];
  link_mark.inclusive = false;
  link_mark.exitable = true;
  link_mark.to_dom = Some(DomOutputSpec::tag("a").attr_of("href", "href").hole());
  link_mark.parse_dom = vec![ParseRule::tag("a[href]").with_get_attrs(|input| {
    let mut a = Attrs::new();
    a.insert("href".into(), Value::String(input.element()?.attr("href").unwrap_or_default().to_owned()));
    Some(a)
  })];

  let mut spec = SchemaSpec::default();
  spec.top_node = Some("doc".into());
  spec.nodes = vec![
    ("doc".into(), doc_spec),
    ("paragraph".into(), paragraph),
    ("heading".into(), heading),
    ("bulleted_list".into(), flat_textblock("div", true, vec![])),
    ("numbered_list".into(), flat_textblock("div", true, vec![])),
    ("todo".into(), todo),
    ("quote".into(), quote),
    ("code_block".into(), code_block),
    ("divider".into(), divider),
    ("math".into(), atom_block("div", vec![str_attr("source", false)])),
    ("mermaid".into(), atom_block("div", vec![str_attr("source", false)])),
    ("board".into(), atom_block("div", vec![str_attr("source", false)])),
    ("image".into(), image),
    (table_name, table_spec),
    table_nodes::table_row(),
    table_nodes::table_header(),
    table_nodes::table_cell(),
    ("text".into(), text_spec),
  ];
  spec.marks = vec![
    starter::marks::bold(),
    starter::marks::italic(),
    starter::marks::strike(),
    ("code".into(), code_mark),
    ("link".into(), link_mark),
    starter::marks::underline(),
  ];
  Schema::new(spec).expect("mica_schema must be valid")
}

// ── 2. Bidirectional Block <-> ProseMirror Conversion ────────────────────────

fn pm_mark_from_core(schema: &Schema, m: &core_marks::Mark) -> Option<PmMark> {
  match m.ty.as_str() {
    "bold" | "italic" | "strike" | "code" | "underline" => schema.mark(&m.ty, None).ok(),
    "link" => {
      let mut a = Attrs::new();
      a.insert("href".into(), Value::String(m.href.clone().unwrap_or_default()));
      schema.mark("link", Some(a)).ok()
    }
    _ => None,
  }
}

fn inline_fragment(schema: &Schema, text: &str, marks: &[core_marks::Mark]) -> Fragment {
  if text.is_empty() {
    return Fragment::empty();
  }
  let total = utf16_len(text) as u32;
  if marks.is_empty() {
    return Fragment::from(vec![schema.text(text, Vec::new())]);
  }
  let mut cuts = vec![0, total];
  for m in marks.iter().filter(|m| m.end > m.start) {
    cuts.extend([m.start.min(total), m.end.min(total)]);
  }
  cuts.sort_unstable();
  cuts.dedup();
  let mut nodes = Vec::new();
  for win in cuts.windows(2) {
    let (a, b) = (win[0], win[1]);
    let (bs, be) = (utf16_to_byte(text, a as usize), utf16_to_byte(text, b as usize));
    if a >= b || bs >= be {
      continue;
    }
    let set = marks
      .iter()
      .filter(|m| m.start <= a && m.end >= b)
      .filter_map(|m| pm_mark_from_core(schema, m))
      .fold(Vec::new(), |acc, pm| pm.add_to_set(&acc));
    nodes.push(schema.text(&text[bs..be], set));
  }
  Fragment::from(nodes)
}

fn text_marks_from_fragment(content: &Fragment) -> (String, Vec<core_marks::Mark>) {
  let (mut text, mut marks, mut offset) = (String::new(), Vec::new(), 0u32);
  let mut open: BTreeMap<String, (u32, Option<String>)> = BTreeMap::new();
  for child in content.iter() {
    let Some(s) = child.text() else { continue };
    let here: BTreeMap<String, Option<String>> = child
      .marks()
      .iter()
      .map(|m| {
        let name = m.ty.name().to_owned();
        let href = (name == "link")
          .then(|| m.attrs.get("href").and_then(Value::as_str).map(str::to_owned))
          .flatten();
        (name, href)
      })
      .collect();
    let closing: Vec<String> = open
      .iter()
      .filter(|(ty, (_, href))| here.get(*ty) != Some(href))
      .map(|(ty, _)| ty.clone())
      .collect();
    for ty in closing {
      if let Some((start, href)) = open.remove(&ty)
        && offset > start
      {
        marks.push(core_marks::Mark { start, end: offset, ty, href, title: None });
      }
    }
    for (ty, href) in here {
      open.entry(ty).or_insert((offset, href));
    }
    text.push_str(s);
    offset += child.utf16_len() as u32;
  }
  for (ty, (start, href)) in open {
    if offset > start {
      marks.push(core_marks::Mark { start, end: offset, ty, href, title: None });
    }
  }
  marks.sort_by(|a, b| (a.start, a.end, a.ty.as_str()).cmp(&(b.start, b.end, b.ty.as_str())));
  (text, marks)
}

fn pm_table_from_block(schema: &Schema, block: &Block) -> Option<Node> {
  let mut rows = Vec::new();
  if let Some(raw_rows) = block.data.get("rows").and_then(Value::as_array) {
    for (r_idx, row_val) in raw_rows.iter().enumerate() {
      let Some(arr) = row_val.as_array() else { continue };
      let cell_kind = if r_idx == 0 { "tableHeader" } else { "tableCell" };
      let mut cells = Vec::with_capacity(arr.len().max(1));
      for cv in arr {
        let dec = decode_cell(cv);
        let marks: Vec<core_marks::Mark> = dec
          .marks
          .iter()
          .filter(|m| m.end > m.start)
          .map(|m| core_marks::Mark {
            start: m.start as u32,
            end: m.end as u32,
            ty: m.kind.clone(),
            href: m.href.clone(),
            title: m.title.clone(),
          })
          .collect();
        let para = schema.node("paragraph", None, inline_fragment(schema, &dec.text, &marks), Vec::new()).ok()?;
        cells.push(schema.node(cell_kind, None, Fragment::from(vec![para]), Vec::new()).ok()?);
      }
      if cells.is_empty() {
        let para = schema.node("paragraph", None, Fragment::empty(), Vec::new()).ok()?;
        cells.push(schema.node(cell_kind, None, Fragment::from(vec![para]), Vec::new()).ok()?);
      }
      rows.push(schema.node("tableRow", None, Fragment::from(cells), Vec::new()).ok()?);
    }
  }
  if rows.is_empty() {
    let t = table_commands::create_table(schema, 2, 2, true)?;
    let mut a = t.attrs().clone();
    a.insert("id".into(), Value::String(block.id.clone()));
    return schema.node("table", Some(a), t.content().clone(), Vec::new()).ok();
  }
  let mut a = make_attrs(block.id.clone(), &[]);
  if let Some(aligns) = block.data.get("aligns").filter(|v| v.is_array()) {
    a.insert("aligns".into(), aligns.clone());
  }
  schema.node("table", Some(a), Fragment::from(rows), Vec::new()).ok()
}

fn pm_node_from_block(schema: &Schema, block: &Block) -> Node {
  let id = if block.id.is_empty() { new_block_id() } else { block.id.clone() };
  let indent = block.data.get("indent").and_then(Value::as_u64).unwrap_or(0).min(MAX_INDENT);
  let marks = core_marks::marks_from_data(&block.data);
  let prose = |kind: &str, extra: &[(&str, Value)]| {
    let mut a = make_attrs(id.clone(), extra);
    a.insert("indent".into(), Value::from(indent));
    schema.node(kind, Some(a), inline_fragment(schema, &block.text, &marks), Vec::new()).ok()
  };
  let source_atom = |kind: &str| {
    let a = make_attrs(id.clone(), &[("source", Value::String(block.text.clone()))]);
    schema.node(kind, Some(a), Fragment::empty(), Vec::new()).ok()
  };

  let built = match block.kind.as_str() {
    "heading" => {
      let lvl = block.data.get("level").and_then(Value::as_u64).unwrap_or(1).clamp(1, 6);
      prose("heading", &[("level", Value::from(lvl))])
    }
    "bulleted_list" | "bullet_list" => prose("bulleted_list", &[]),
    "numbered_list" | "number_list" => prose("numbered_list", &[]),
    "todo" => prose("todo", &[("checked", Value::Bool(block.data.get("checked").and_then(Value::as_bool).unwrap_or(false)))]),
    "quote" => prose("quote", &[]),
    "code_block" | "code" => {
      let lang = block.data.get("language").and_then(Value::as_str).unwrap_or_default();
      if lang.eq_ignore_ascii_case("mermaid") {
        source_atom("mermaid")
      } else if lang.eq_ignore_ascii_case("mica-board") || lang.eq_ignore_ascii_case("board") {
        source_atom("board")
      } else {
        let a = make_attrs(id.clone(), &[("language", Value::String(lang.to_owned()))]);
        let frag = if block.text.is_empty() { Fragment::empty() } else { Fragment::from(vec![schema.text(&block.text, Vec::new())]) };
        schema.node("code_block", Some(a), frag, Vec::new()).ok()
      }
    }
    "divider" => schema.node("divider", Some(make_attrs(id.clone(), &[])), Fragment::empty(), Vec::new()).ok(),
    "math_block" | "math" => source_atom("math"),
    "mermaid" => source_atom("mermaid"),
    "board" => source_atom("board"),
    "image" => {
      let src = block.data.get("url").and_then(Value::as_str)
        .or_else(|| block.data.get("name").and_then(Value::as_str))
        .or_else(|| block.data.get("file_id").and_then(Value::as_str))
        .unwrap_or_default();
      let a = make_attrs(id.clone(), &[("src", Value::String(src.to_owned())), ("alt", Value::String(block.text.clone()))]);
      schema.node("image", Some(a), Fragment::empty(), Vec::new()).ok()
    }
    "table" => pm_table_from_block(schema, block),
    _ => prose("paragraph", &[]),
  };
  built.unwrap_or_else(|| {
    schema.node("paragraph", Some(make_attrs(id, &[])), Fragment::empty(), Vec::new()).expect("paragraph builds")
  })
}

fn pm_doc_from_blocks(schema: &Schema, blocks: &[Block]) -> Node {
  let children = if blocks.is_empty() {
    vec![schema.node("paragraph", Some(make_attrs(new_block_id(), &[])), Fragment::empty(), Vec::new()).expect("paragraph builds")]
  } else {
    blocks.iter().map(|b| pm_node_from_block(schema, b)).collect()
  };
  schema.node("doc", None, Fragment::from(children), Vec::new()).expect("doc builds")
}

fn extract_cell_inline(cell: &Node) -> InlineText {
  let (mut text, mut marks, mut offset) = (String::new(), Vec::new(), 0usize);
  for (idx, child) in cell.content().iter().enumerate() {
    if idx > 0 {
      text.push('\n');
      offset += 1;
    }
    let (seg, seg_marks) = text_marks_from_fragment(child.content());
    for m in seg_marks.into_iter().filter(|m| m.end > m.start) {
      marks.push(MdMark { start: m.start as usize + offset, end: m.end as usize + offset, kind: m.ty, href: m.href, title: m.title });
    }
    offset += utf16_len(&seg);
    text.push_str(&seg);
  }
  marks.sort_by(|a, b| (a.start, a.end, a.kind.as_str()).cmp(&(b.start, b.end, b.kind.as_str())));
  InlineText { text, marks }
}

fn block_from_pm_node(node: &Node, id: String) -> Block {
  let attrs = node.attrs();
  let indent = attr_u64(attrs, "indent", 0);
  let prose = |kind: &str, mut data: Map<String, Value>| {
    let (text, marks) = text_marks_from_fragment(node.content());
    if indent > 0 {
      data.insert("indent".into(), Value::from(indent));
    }
    if !marks.is_empty() {
      data.insert("marks".into(), core_marks::marks_to_json(&marks));
    }
    Block { id: id.clone(), kind: kind.to_owned(), text, data: if data.is_empty() { Value::Null } else { Value::Object(data) }, children: Vec::new() }
  };

  match node.ty().name() {
    "paragraph" => prose("paragraph", Map::new()),
    "heading" => {
      let mut d = Map::new();
      d.insert("level".into(), Value::from(attr_u64(attrs, "level", 1).clamp(1, 6)));
      prose("heading", d)
    }
    "bulleted_list" => prose("bulleted_list", Map::new()),
    "numbered_list" => prose("numbered_list", Map::new()),
    "todo" => {
      let mut d = Map::new();
      d.insert("checked".into(), Value::Bool(attr_bool(attrs, "checked")));
      prose("todo", d)
    }
    "quote" => prose("quote", Map::new()),
    "code_block" => {
      let lang = attr_str(attrs, "language");
      let data = if lang.is_empty() { Value::Null } else { json!({ "language": lang }) };
      Block::new(id, "code_block").with_text(node.text_content()).with_data(data)
    }
    "divider" => Block::new(id, "divider"),
    "math" => Block::new(id, "math_block").with_text(attr_str(attrs, "source")),
    "mermaid" => Block::new(id, "code_block").with_text(attr_str(attrs, "source")).with_data(json!({ "language": "mermaid" })),
    "board" => Block::new(id, "code_block").with_text(attr_str(attrs, "source")).with_data(json!({ "language": "mica-board" })),
    "image" => {
      let src = attr_str(attrs, "src");
      let data = if src.is_empty() { Value::Null } else { json!({ "url": src }) };
      Block::new(id, "image").with_text(attr_str(attrs, "alt")).with_data(data)
    }
    "table" => {
      let rows: Vec<Value> = node.content().iter().map(|r| {
        Value::Array(r.content().iter().map(|c| encode_cell(&extract_cell_inline(c))).collect())
      }).collect();
      let mut d = Map::new();
      d.insert("rows".into(), Value::Array(rows));
      if let Some(aligns) = attrs.get("aligns").filter(|v| v.is_array()) {
        d.insert("aligns".into(), aligns.clone());
      }
      Block::new(id, "table").with_data(Value::Object(d))
    }
    _ => prose("paragraph", Map::new()),
  }
}

fn blocks_from_pm_doc(doc: &Node) -> Vec<Block> {
  let mut seen: HashSet<String> = HashSet::with_capacity(doc.child_count());
  doc
    .content()
    .iter()
    .map(|child| {
      let raw = attr_str(child.attrs(), "id");
      let id = if raw.is_empty() || !seen.insert(raw.to_owned()) {
        let fresh = new_block_id();
        seen.insert(fresh.clone());
        fresh
      } else {
        raw.to_owned()
      };
      block_from_pm_node(child, id)
    })
    .collect()
}

fn parse_markdown_to_doc(markdown: &str) -> (MicaDoc, String, Vec<Block>) {
  let mut parsed = parse(markdown, &ParseOptions::default());
  let root_id = if parsed.root_block_id.is_empty() { DEFAULT_ROOT_ID.to_owned() } else { parsed.root_block_id.clone() };
  let root_idx = parsed.blocks.iter().position(|b| b.id == root_id).unwrap_or_else(|| {
    parsed.blocks.insert(0, Block::new(root_id.clone(), "page"));
    0
  });
  if parsed.blocks[root_idx].children.is_empty() {
    let p_id = new_block_id();
    parsed.blocks[root_idx].children.push(p_id.clone());
    parsed.blocks.push(Block::new(p_id, "paragraph"));
  }
  let doc = MicaDoc::from_blocks(&root_id, &parsed.blocks);
  let body = body_blocks_from_doc(&doc);
  (doc, root_id, body)
}

fn body_blocks_from_doc(doc: &MicaDoc) -> Vec<Block> {
  let root_id = doc.root_block_id();
  let all = doc.to_blocks();
  let by_id: HashMap<&str, &Block> = all.iter().map(|b| (b.id.as_str(), b)).collect();
  let Some(root) = by_id.get(root_id.as_str()) else { return Vec::new() };
  root.children.iter().filter_map(|cid| {
    by_id.get(cid.as_str()).map(|b| {
      let mut flat = (*b).clone();
      flat.children.clear();
      flat
    })
  }).collect()
}

fn doc_snapshot(doc: &MicaDoc) -> Document {
  Document { schema_version: SCHEMA_VERSION, root_block_id: doc.root_block_id(), blocks: doc.to_blocks() }
}
fn serialize_doc_markdown(doc: &MicaDoc) -> String {
  serialize(&doc_snapshot(doc), &SerializeOptions::default()).unwrap_or_default()
}
fn render_doc_html(doc: &MicaDoc) -> String {
  render_html(&doc_snapshot(doc), &HtmlOptions::default()).unwrap_or_default()
}

/// Applies ONLY changed/inserted/moved/deleted block ops from `prev` to `next`.
/// Caller wraps in `begin_history_batch()` / `end_history_batch()`.
fn apply_block_diff(doc: &mut MicaDoc, root_id: &str, prev: &[Block], next: &[Block]) {
  let prev_map: HashMap<&str, &Block> = prev.iter().map(|b| (b.id.as_str(), b)).collect();
  let next_ids: HashSet<&str> = next.iter().map(|b| b.id.as_str()).collect();
  let mut order: Vec<String> = Vec::with_capacity(prev.len());
  for b in prev {
    if !next_ids.contains(b.id.as_str()) {
      doc.delete_block(&b.id, false);
    } else {
      order.push(b.id.clone());
    }
  }
  for (target_idx, next_b) in next.iter().enumerate() {
    match prev_map.get(next_b.id.as_str()) {
      None => {
        doc.insert_block(root_id, target_idx, next_b);
        order.insert(target_idx.min(order.len()), next_b.id.clone());
      }
      Some(prev_b) => {
        if order.get(target_idx).map(String::as_str) != Some(next_b.id.as_str())
          && let Some(cur) = order.iter().position(|id| id == &next_b.id)
        {
          doc.move_block(&next_b.id, root_id, target_idx);
          let moved = order.remove(cur);
          order.insert(target_idx.min(order.len()), moved);
        }
        if *prev_b != next_b {
          let kind = (prev_b.kind != next_b.kind).then_some(next_b.kind.as_str());
          let text_changed = prev_b.text != next_b.text;
          let data_changed = prev_b.data != next_b.data;
          // Always pass `Some(&next_b.data)` when text/data changed so `set_block_text` keeps marks.
          doc.update_block_fields(
            &next_b.id,
            kind,
            text_changed.then_some(next_b.text.as_str()),
            (text_changed || data_changed).then_some(&next_b.data),
          );
        }
      }
    }
  }
}

// ── 3. Flat-Block Input Rules & Keymap (Zero Duplicate History) ──────────────

struct MicaInputRulesPlugin {
  key: PluginKey,
}

impl MicaInputRulesPlugin {
  fn plugin() -> Plugin {
    Arc::new(Self { key: PluginKey::new("micaInputRules") })
  }

  fn try_convert(state: &EditorState, from: usize, to: usize, trigger: &str) -> Option<Transaction> {
    let res = state.doc().resolve(from).ok()?;
    if res.depth != 1
      || res.parent().ty().name() != "paragraph"
      || res.marks().iter().any(|m| m.ty.spec().code)
    {
      return None;
    }
    let parent = res.parent();
    let before = parent.content().text_between(0, res.parent_offset, None, Some("\u{fffc}"));
    let block_start = res.start(1);
    let id = attr_str(parent.attrs(), "id").to_owned();
    let indent = attr_u64(parent.attrs(), "indent", 0);
    let schema = state.schema();

    let convert = |target: &str, extra: &[(&str, Value)]| -> Option<Transaction> {
      let ty = schema.node_type(target)?;
      let mut a = make_attrs(id.clone(), extra);
      if target != "code_block" {
        a.insert("indent".into(), Value::from(indent));
      }
      let mut tr = state.tr();
      let _ = tr.delete(block_start, to);
      tr.set_block_type(block_start, block_start, &ty, Some(a)).ok()?;
      tr.scroll_into_view();
      Some(tr)
    };
    let divider = || -> Option<Transaction> {
      let node = schema.node("divider", Some(make_attrs(new_block_id(), &[])), Fragment::empty(), Vec::new()).ok()?;
      let para_pos = res.before(1);
      let mut tr = state.tr();
      let _ = tr.delete(block_start, to);
      tr.insert_node(para_pos, node).ok()?;
      if let Ok(at) = tr.doc().resolve(para_pos + 2) {
        tr.set_selection(Selection::near(&at, 1));
      }
      tr.scroll_into_view();
      Some(tr)
    };

    if trigger == "-" && before == "--" && parent.content().size() == 2 {
      return divider();
    }
    if matches!(trigger, " " | "\n") {
      if before == "---" {
        return divider();
      }
      if let Some(lang) = before.strip_prefix("```")
        && lang.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'+' | b'#'))
      {
        return convert("code_block", &[("language", Value::String(lang.to_owned()))]);
      }
    }
    if trigger != " " {
      return None;
    }
    if !before.is_empty() && before.len() <= 6 && before.bytes().all(|b| b == b'#') {
      return convert("heading", &[("level", Value::from(before.len() as u64))]);
    }
    if matches!(before.as_str(), "-" | "*" | "+") {
      return convert("bulleted_list", &[]);
    }
    if let Some(num) = before.strip_suffix('.')
      && !num.is_empty()
      && num.bytes().all(|b| b.is_ascii_digit())
    {
      return convert("numbered_list", &[]);
    }
    if matches!(before.as_str(), "[]" | "[ ]" | "[x]" | "[X]") {
      return convert("todo", &[("checked", Value::Bool(matches!(before.as_str(), "[x]" | "[X]")))]);
    }
    if before == ">" {
      return convert("quote", &[]);
    }
    None
  }
}

impl PluginSpec for MicaInputRulesPlugin {
  fn key(&self) -> PluginKey {
    self.key.clone()
  }
  fn handle_text_input(
    &self,
    state: &EditorState,
    from: usize,
    to: usize,
    text: &str,
    dispatch: &mut dyn FnMut(Transaction),
  ) -> bool {
    if let Some(tr) = Self::try_convert(state, from, to, text) {
      dispatch(tr);
      return true;
    }
    false
  }
  fn handle_key_down(
    &self,
    state: &EditorState,
    event: &KeyEvent,
    dispatch: &mut dyn FnMut(Transaction),
  ) -> bool {
    if event.key == "Enter"
      && !event.ctrl
      && !event.meta
      && !event.alt
      && !event.shift
      && let Some(cursor) = state.selection().cursor()
      && let Some(tr) = Self::try_convert(state, cursor.pos, cursor.pos, "\n")
    {
      dispatch(tr);
      return true;
    }
    false
  }
}

fn reset_to_paragraph(state: &EditorState, pos: usize, id: String) -> Option<Transaction> {
  let ty = state.schema().node_type("paragraph")?;
  let mut tr = state.tr();
  tr.set_block_type(pos, pos, &ty, Some(make_attrs(id, &[("indent", Value::from(0))]))).ok()?;
  tr.scroll_into_view();
  Some(tr)
}

fn mica_enter_cmd(state: &EditorState, dispatch: Dispatch<'_>) -> bool {
  let sel = state.selection();
  let from = sel.resolved_from();
  if from.depth != 1 {
    return false;
  }
  let parent = from.parent();
  if !matches!(parent.ty().name(), "bulleted_list" | "numbered_list" | "todo" | "quote") {
    return false;
  }
  if sel.empty() && parent.content().size() == 0 {
    let indent = attr_u64(parent.attrs(), "indent", 0);
    if let Some(dispatch) = dispatch {
      if indent > 0 {
        let mut tr = state.tr();
        let _ = tr.set_node_attribute(from.before(1), "indent", Value::from(indent - 1));
        tr.scroll_into_view();
        dispatch(tr);
      } else if let Some(tr) = reset_to_paragraph(state, from.pos, attr_str(parent.attrs(), "id").to_owned()) {
        dispatch(tr);
      }
    }
    return true;
  }
  let ty = parent.ty().clone();
  let split_as: SplitAs = Arc::new(move |node, _, _| Some((ty.clone(), Some(node.ty().split_attrs(node.attrs())))));
  split_block_as(Some(split_as))(state, dispatch)
}

fn mica_backspace_cmd(state: &EditorState, dispatch: Dispatch<'_>) -> bool {
  let Some(cursor) = state.selection().cursor() else { return false };
  if cursor.depth != 1 || cursor.parent_offset != 0 || undo_input_rule(state, None) {
    return false;
  }
  let parent = cursor.parent();
  let name = parent.ty().name();
  let indent = attr_u64(parent.attrs(), "indent", 0);
  if indent > 0 && is_indentable_kind(name) {
    if let Some(dispatch) = dispatch {
      let mut tr = state.tr();
      let _ = tr.set_node_attribute(cursor.before(1), "indent", Value::from(indent - 1));
      tr.scroll_into_view();
      dispatch(tr);
    }
    return true;
  }
  if matches!(name, "heading" | "bulleted_list" | "numbered_list" | "todo" | "quote") {
    if let Some(dispatch) = dispatch
      && let Some(tr) = reset_to_paragraph(state, cursor.pos, attr_str(parent.attrs(), "id").to_owned())
    {
      dispatch(tr);
    }
    return true;
  }
  false
}

fn mica_indent_cmd(delta: i64) -> BoxCommand {
  command(move |state, dispatch| {
    if tables::is_in_table(state) {
      return table_commands::go_to_next_cell(if delta > 0 { 1 } else { -1 })(state, dispatch);
    }
    let sel = state.selection();
    if delta > 0 && sel.resolved_from().parent().ty().name() == "code_block" {
      if let Some(dispatch) = dispatch {
        let mut tr = state.tr();
        tr.insert_text("  ", None, None);
        tr.scroll_into_view();
        dispatch(tr);
      }
      return true;
    }
    let mut targets: Vec<(usize, u64)> = Vec::new();
    for range in sel.ranges() {
      state.doc().nodes_between(range.from.pos, range.to.pos, &mut |node, pos, parent, _| {
        if parent.is_some_and(|p| p.ty().name() == "doc") && is_indentable_kind(node.ty().name()) {
          let cur = attr_u64(node.attrs(), "indent", 0);
          let next = if delta > 0 { (cur + 1).min(MAX_INDENT) } else { cur.saturating_sub(1) };
          if next != cur && !targets.iter().any(|(p, _)| *p == pos) {
            targets.push((pos, next));
          }
          return false;
        }
        true
      });
    }
    if targets.is_empty() {
      return false;
    }
    if let Some(dispatch) = dispatch {
      let mut tr = state.tr();
      for (pos, next) in targets {
        let _ = tr.set_node_attribute(pos, "indent", Value::from(next));
      }
      tr.scroll_into_view();
      dispatch(tr);
    }
    true
  })
}

fn history_key_cmd(handle: ThreadBoundWeak, is_redo: bool) -> BoxCommand {
  command(move |_, dispatch| {
    let Some(bridge) = handle.upgrade() else { return false };
    let can = if is_redo { bridge.doc.borrow().can_redo() } else { bridge.doc.borrow().can_undo() };
    if !can {
      return false;
    }
    if dispatch.is_some() {
      bridge.apply_history_step(is_redo);
    }
    true
  })
}

fn build_plugins(schema: &Schema, weak: ThreadBoundWeak) -> Vec<Plugin> {
  let mut plugins: Vec<Plugin> = vec![MicaInputRulesPlugin::plugin()];

  let inline_rules: Vec<_> = ["bold", "italic", "strike", "code"]
    .into_iter()
    .flat_map(|name| kit::mark_rules(schema, name))
    .collect();
  if !inline_rules.is_empty() {
    plugins.push(input_rules(inline_rules));
  }

  let redo_cmd = history_key_cmd(weak.clone(), true);
  let mut bindings: Vec<(String, BoxCommand)> = vec![
    ("Mod-z".into(), history_key_cmd(weak, false)),
    ("Mod-y".into(), redo_cmd.clone()),
    ("Shift-Mod-z".into(), redo_cmd),
    ("Enter".into(), command(mica_enter_cmd)),
    ("Backspace".into(), command(mica_backspace_cmd)),
    ("Tab".into(), mica_indent_cmd(1)),
    ("Shift-Tab".into(), mica_indent_cmd(-1)),
  ];
  for (keys, mark_name) in [
    (&["Mod-b", "Mod-B"][..], "bold"),
    (&["Mod-i", "Mod-I"][..], "italic"),
    (&["Mod-e", "Mod-E"][..], "code"),
    (&["Mod-u", "Mod-U"][..], "underline"),
    (&["Mod-Shift-s"][..], "strike"),
  ] {
    if let Some(ty) = schema.mark_type(mark_name) {
      let cmd = toggle_mark(ty, None, true);
      bindings.extend(keys.iter().map(|k| ((*k).to_owned(), cmd.clone())));
    }
  }
  plugins.push(keymap(bindings));
  plugins.push(table_editing(TableEditingOptions::default()));
  plugins.push(fix_tables_plugin());
  // NOTE: `rich_editor::extensions::history` is intentionally NOT installed!
  plugins.push(keymap(core_keymap::core_keymap()));

  let mut trailing_opts = TrailingNodeOptions::default();
  trailing_opts.node = Some("paragraph".into());
  trailing_opts.not_after = ["paragraph", "heading", "bulleted_list", "numbered_list", "todo", "quote"]
    .into_iter()
    .map(str::to_owned)
    .collect();
  plugins.push(trailing_node(trailing_opts));
  plugins
}

// ── 4. Thread-Checked Weak Handle for EditorSession Hooks ────────────────────

#[derive(Clone)]
struct ThreadBoundWeak {
  thread: ThreadId,
  inner: Arc<ThreadBoundWeakInner>,
}

struct ThreadBoundWeakInner {
  thread: ThreadId,
  weak: ManuallyDrop<Weak<MicaDocumentBridge>>,
}

// SAFETY: `weak` is ONLY upgraded or dropped on `self.thread` (the single NSH
// engine/UI thread or test thread where `MicaDocumentBridge` lives).
unsafe impl Send for ThreadBoundWeakInner {}
unsafe impl Sync for ThreadBoundWeakInner {}

impl Drop for ThreadBoundWeakInner {
  fn drop(&mut self) {
    if std::thread::current().id() == self.thread {
      // SAFETY: Same thread that created `weak`, dropped at most once.
      unsafe { ManuallyDrop::drop(&mut self.weak) };
    }
  }
}

impl ThreadBoundWeak {
  fn from_weak(weak: Weak<MicaDocumentBridge>) -> Self {
    let thread = std::thread::current().id();
    Self { thread, inner: Arc::new(ThreadBoundWeakInner { thread, weak: ManuallyDrop::new(weak) }) }
  }
  fn upgrade(&self) -> Option<Rc<MicaDocumentBridge>> {
    (std::thread::current().id() == self.thread).then(|| self.inner.weak.upgrade()).flatten()
  }
}

// ── 5. Signal Derivation Helpers ─────────────────────────────────────────────

fn compute_outline(doc: &Node) -> Vec<OutlineItem> {
  let mut items = Vec::new();
  doc.for_each(&mut |node, pos, block_index| {
    if node.ty().name() == "heading" {
      let text = node.text_content();
      if !text.trim().is_empty() {
        items.push(OutlineItem {
          block_index,
          pos,
          level: attr_u64(node.attrs(), "level", 1).clamp(1, 6),
          text: text.trim().to_owned(),
        });
      }
    }
  });
  items
}

fn compute_word_count_label(doc: &Node) -> String {
  let (mut chars, mut words) = (0usize, 0usize);
  doc.for_each(&mut |node, _, _| {
    let raw = match node.ty().name() {
      "math" | "mermaid" | "board" => attr_str(node.attrs(), "source").to_owned(),
      "image" => attr_str(node.attrs(), "alt").to_owned(),
      _ => node.text_content(),
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
      return;
    }
    chars += trimmed.chars().filter(|c| !c.is_whitespace()).count();
    let mut in_ascii = false;
    for ch in trimmed.chars() {
      if ch.is_ascii_alphanumeric() {
        if !in_ascii {
          words += 1;
          in_ascii = true;
        }
      } else {
        in_ascii = false;
        if !ch.is_whitespace() && !ch.is_ascii_punctuation() {
          words += 1;
        }
      }
    }
  });
  format!("{words} 词 · {chars} 字")
}

fn compute_active_state(state: &EditorState) -> (String, u64, Vec<String>) {
  let sel = state.selection();
  let node = sel.as_node().map(|ns| ns.node()).or_else(|| {
    let from = sel.resolved_from();
    (from.depth >= 1).then(|| from.node(1))
  });
  let (kind, level) = match node {
    Some(n) => {
      let name = n.ty().name().to_owned();
      let lvl = if name == "heading" { attr_u64(n.attrs(), "level", 1) } else { 0 };
      (name, lvl)
    }
    None => ("paragraph".into(), 0),
  };
  let marks = MARK_NAMES
    .iter()
    .filter(|&&m| state.schema().mark_type(m).is_some_and(|ty| is_mark_active(state, &ty)))
    .map(|&m| m.to_owned())
    .collect();
  (kind, level, marks)
}

fn ensure_unique_block_ids(state: &EditorState) -> Option<Transaction> {
  let mut seen: HashSet<String> = HashSet::with_capacity(state.doc().child_count());
  let mut fixups: Vec<(usize, String)> = Vec::new();
  state.doc().for_each(&mut |node, pos, _| {
    let cur = attr_str(node.attrs(), "id");
    if cur.is_empty() || !seen.insert(cur.to_owned()) {
      let fresh = new_block_id();
      seen.insert(fresh.clone());
      fixups.push((pos, fresh));
    }
  });
  if fixups.is_empty() {
    return None;
  }
  let mut tr = state.tr();
  for (pos, fresh_id) in fixups {
    let _ = tr.set_node_attribute(pos, "id", Value::String(fresh_id));
  }
  tr.set_meta(SKIP_TRAILING_NODE, Value::Bool(true));
  Some(tr)
}

// ── 6. MicaDocumentBridge ────────────────────────────────────────────────────

type PersistCallback = Rc<dyn Fn(String, Option<Vec<u8>>)>;

/// Bridges `mica_core::MicaDoc` (yrs CRDT, sole storage & Undo/Redo truth)
/// with `nsh_base::rich_editor::view::EditorSession` and reactive UI signals.
pub struct MicaDocumentBridge {
  schema: Schema,
  session: EditorSession,
  doc: Rc<RefCell<MicaDoc>>,
  root_id: RefCell<String>,
  prev_blocks: RefCell<Vec<Block>>,
  on_persist: RefCell<Option<PersistCallback>>,
  word_count: Signal<String>,
  block_count: Signal<usize>,
  outline: Signal<Vec<OutlineItem>>,
  can_undo: Signal<bool>,
  can_redo: Signal<bool>,
  active_kind: Signal<String>,
  active_level: Signal<u64>,
  active_marks: Signal<Vec<String>>,
  last_edited: Signal<String>,
  session_rev: Signal<u64>,
}

impl Drop for MicaDocumentBridge {
  fn drop(&mut self) {
    self.session.set_dispatch_transaction(None);
  }
}

impl MicaDocumentBridge {
  /// Constructs a live [`MicaDocumentBridge`] initialized from `markdown`.
  pub fn from_markdown(markdown: &str) -> Rc<Self> {
    let (mica_doc, root_id, body_blocks) = parse_markdown_to_doc(markdown);
    Self::from_parts(mica_doc, root_id, body_blocks)
  }

  /// Constructs a live [`MicaDocumentBridge`] from an encoded yrs CRDT state update.
  pub fn from_update(bytes: &[u8]) -> Result<Rc<Self>, String> {
    let mica_doc = MicaDoc::from_update(bytes).map_err(|e| e.to_string())?;
    let root_id = mica_doc.root_block_id();
    let body_blocks = body_blocks_from_doc(&mica_doc);
    Ok(Self::from_parts(mica_doc, root_id, body_blocks))
  }

  fn from_parts(mica_doc: MicaDoc, root_id: String, body_blocks: Vec<Block>) -> Rc<Self> {
    let schema = mica_schema();
    let pm_doc = pm_doc_from_blocks(&schema, &body_blocks);
    let initial_prev = blocks_from_pm_doc(&pm_doc);

    let bridge = Rc::new_cyclic(|weak| {
      let plugins = build_plugins(&schema, ThreadBoundWeak::from_weak(weak.clone()));
      let state = EditorState::create(
        EditorStateConfig::new(schema.clone()).doc(pm_doc).plugins(plugins),
      )
      .expect("initial EditorState must be valid");
      let (act_kind, act_level, act_marks) = compute_active_state(&state);
      let word_count = signal(compute_word_count_label(state.doc()));
      let block_count = signal(state.doc().child_count());
      let outline = signal(compute_outline(state.doc()));
      let session = EditorSession::new(state, NodeViews::new());

      Self {
        schema,
        session,
        doc: Rc::new(RefCell::new(mica_doc)),
        root_id: RefCell::new(root_id),
        prev_blocks: RefCell::new(initial_prev),
        on_persist: RefCell::new(None),
        word_count,
        block_count,
        outline,
        can_undo: signal(false),
        can_redo: signal(false),
        active_kind: signal(act_kind),
        active_level: signal(act_level),
        active_marks: signal(act_marks),
        last_edited: signal(String::from(DEFAULT_STATUS_TEXT)),
        session_rev: signal(0u64),
      }
    });

    let dispatch_handle = ThreadBoundWeak::from_weak(Rc::downgrade(&bridge));
    bridge.session.set_dispatch_transaction(Some(Arc::new(move |session, tr| {
      if let Some(b) = dispatch_handle.upgrade() {
        b.handle_dispatch(session, tr);
      }
    })));
    bridge
  }

  pub fn session(&self) -> EditorSession {
    self.session.clone()
  }

  pub fn set_on_persist(&self, cb: Rc<dyn Fn(String, Option<Vec<u8>>)>) {
    *self.on_persist.borrow_mut() = Some(cb);
  }

  pub fn load_markdown(&self, markdown: &str) {
    let (mica_doc, root_id, body_blocks) = parse_markdown_to_doc(markdown);
    let pm_doc = pm_doc_from_blocks(&self.schema, &body_blocks);
    *self.doc.borrow_mut() = mica_doc;
    *self.root_id.borrow_mut() = root_id;
    *self.prev_blocks.borrow_mut() = blocks_from_pm_doc(&pm_doc);
    self.replace_session_doc(pm_doc, TransactionOrigin::Remote("load".into()), None);
    let state = self.session.state();
    self.sync_signals(&state, true);
    self.last_edited.set(String::from(DEFAULT_STATUS_TEXT));
    self.session_rev.update(|r| *r += 1);
  }

  pub fn apply_remote_update(&self, update: &[u8]) -> Result<(), String> {
    let body_blocks = {
      let mut doc = self.doc.borrow_mut();
      doc.apply_update(update).map_err(|e| e.to_string())?;
      body_blocks_from_doc(&doc)
    };
    let pm_doc = pm_doc_from_blocks(&self.schema, &body_blocks);
    *self.prev_blocks.borrow_mut() = blocks_from_pm_doc(&pm_doc);
    let prev_pos = self.session.state().selection().head();
    self.replace_session_doc(pm_doc, TransactionOrigin::Remote("sync".into()), Some(prev_pos));
    let state = self.session.state();
    self.sync_signals(&state, true);
    Ok(())
  }

  pub fn undo(&self) {
    self.apply_history_step(false);
  }

  pub fn redo(&self) {
    self.apply_history_step(true);
  }

  pub fn toggle_mark(&self, mark_name: &str) {
    if let Some(ty) = self.schema.mark_type(mark_name) {
      self.session.run(&toggle_mark(ty, None, true));
    }
  }

  pub fn set_block_type(&self, kind: &str, level: Option<u64>) {
    let (canonical, inferred) = match kind {
      "h1" | "heading1" => ("heading", Some(1)),
      "h2" | "heading2" => ("heading", Some(2)),
      "h3" | "heading3" => ("heading", Some(3)),
      "h4" | "heading4" => ("heading", Some(4)),
      "h5" | "heading5" => ("heading", Some(5)),
      "h6" | "heading6" => ("heading", Some(6)),
      "bullet_list" => ("bulleted_list", None),
      "number_list" => ("numbered_list", None),
      "code" => ("code_block", None),
      other => (other, None),
    };
    let lvl = level.or(inferred).unwrap_or(1).clamp(1, 6);
    match canonical {
      "divider" => {
        self.insert_divider();
        return;
      }
      "table" => {
        self.insert_table(3, 3);
        return;
      }
      "math" | "math_block" | "mermaid" | "board" | "image" => {
        self.insert_custom_block(canonical, "", None);
        return;
      }
      _ => {}
    }
    let Some(ty) = self.schema.node_type(canonical).filter(|t| t.is_textblock()) else { return };
    let state = self.session.state();
    let mut tr = state.tr();
    for range in state.selection().ranges() {
      let _ = tr.set_block_type_with(range.from.pos, range.to.pos, &ty, &|old| {
        let id = attr_str(old.attrs(), "id").to_owned();
        let indent = attr_u64(old.attrs(), "indent", 0);
        Some(match canonical {
          "heading" => make_attrs(id, &[("level", Value::from(lvl)), ("indent", Value::from(indent))]),
          "todo" => make_attrs(id, &[("checked", Value::Bool(attr_bool(old.attrs(), "checked"))), ("indent", Value::from(indent))]),
          "code_block" => make_attrs(id, &[("language", Value::String(attr_str(old.attrs(), "language").to_owned()))]),
          _ => make_attrs(id, &[("indent", Value::from(indent))]),
        })
      });
    }
    if tr.doc_changed() {
      tr.scroll_into_view();
      let _ = self.session.dispatch(tr);
    }
  }

  pub fn insert_divider(&self) {
    if let Ok(node) = self.schema.node("divider", Some(make_attrs(new_block_id(), &[])), Fragment::empty(), Vec::new()) {
      self.insert_block_node(node, true);
    }
  }

  pub fn insert_table(&self, rows: usize, cols: usize) {
    let Some(raw) = table_commands::create_table(&self.schema, rows.max(1), cols.max(1), true) else { return };
    let mut a = raw.attrs().clone();
    a.insert("id".into(), Value::String(new_block_id()));
    if let Ok(node) = self.schema.node("table", Some(a), raw.content().clone(), Vec::new()) {
      self.insert_block_node(node, false);
    }
  }

  pub fn insert_custom_block(&self, kind: &str, text: &str, language: Option<&str>) {
    if kind == "table" {
      self.insert_table(3, 3);
      return;
    }
    let id = new_block_id();
    let frag = || if text.is_empty() { Fragment::empty() } else { Fragment::from(vec![self.schema.text(text, Vec::new())]) };
    let atom = |ty: &str, extra: &[(&str, Value)]| {
      self.schema.node(ty, Some(make_attrs(id.clone(), extra)), Fragment::empty(), Vec::new()).ok()
    };
    let (node_opt, after) = match kind {
      "divider" => (atom("divider", &[]), true),
      "math" | "math_block" => (atom("math", &[("source", Value::String(text.to_owned()))]), true),
      "mermaid" => (atom("mermaid", &[("source", Value::String(text.to_owned()))]), true),
      "board" | "mica-board" => (atom("board", &[("source", Value::String(text.to_owned()))]), true),
      "image" => {
        let (src, alt) = match language { Some(u) if !u.is_empty() => (u, text), _ => (text, "") };
        (atom("image", &[("src", Value::String(src.to_owned())), ("alt", Value::String(alt.to_owned()))]), true)
      }
      "code_block" | "code" => (
        self.schema.node("code_block", Some(make_attrs(id, &[("language", Value::String(language.unwrap_or_default().to_owned()))])), frag(), Vec::new()).ok(),
        false,
      ),
      other => {
        let target = match other {
          "bullet_list" => "bulleted_list",
          "number_list" => "numbered_list",
          k if self.schema.node_type(k).is_some_and(|t| t.is_textblock()) => k,
          _ => "paragraph",
        };
        (self.schema.node(target, Some(make_attrs(id, &[])), frag(), Vec::new()).ok(), false)
      }
    };
    if let Some(node) = node_opt {
      self.insert_block_node(node, after);
    }
  }

  pub fn focus_pos(&self, pos: usize) {
    let state = self.session.state();
    let clamped = pos.min(state.doc().content().size());
    let sel = state.doc().resolve(clamped).map(|r| Selection::near(&r, 1)).unwrap_or_else(|_| Selection::at_start(state.doc()));
    let mut tr = state.tr();
    tr.set_selection(sel);
    tr.scroll_into_view();
    tr.set_meta(FOCUS_META, Value::Bool(true));
    let _ = self.session.dispatch(tr);
  }

  pub fn export_markdown(&self) -> String {
    serialize_doc_markdown(&self.doc.borrow())
  }
  pub fn export_html(&self) -> String {
    render_doc_html(&self.doc.borrow())
  }
  pub fn word_count_signal(&self) -> Signal<String> {
    self.word_count
  }
  pub fn block_count_signal(&self) -> Signal<usize> {
    self.block_count
  }
  pub fn outline_signal(&self) -> Signal<Vec<OutlineItem>> {
    self.outline
  }
  pub fn can_undo_signal(&self) -> Signal<bool> {
    self.can_undo
  }
  pub fn can_redo_signal(&self) -> Signal<bool> {
    self.can_redo
  }
  pub fn active_kind_signal(&self) -> Signal<String> {
    self.active_kind
  }
  pub fn active_level_signal(&self) -> Signal<u64> {
    self.active_level
  }
  pub fn active_marks_signal(&self) -> Signal<Vec<String>> {
    self.active_marks
  }
  pub fn last_edited_signal(&self) -> Signal<String> {
    self.last_edited
  }
  pub fn set_status_flash(&self, msg: impl Into<String>) {
    self.last_edited.set(msg.into());
  }
  pub fn session_rev_signal(&self) -> Signal<u64> {
    self.session_rev
  }

  // ── Internal Sync Helpers ────────────────────────────────────────────────

  fn apply_history_step(&self, is_redo: bool) {
    let (body_blocks, markdown, crdt_update) = {
      let mut doc = self.doc.borrow_mut();
      let before_sv = doc.state_vector();
      let changed = if is_redo { doc.redo() } else { doc.undo() };
      if !changed {
        return;
      }
      let diff = doc.encode_diff(&before_sv).ok().filter(|u| !u.is_empty());
      (body_blocks_from_doc(&doc), serialize_doc_markdown(&doc), diff)
    };
    let pm_doc = pm_doc_from_blocks(&self.schema, &body_blocks);
    *self.prev_blocks.borrow_mut() = blocks_from_pm_doc(&pm_doc);
    let prev_pos = self.session.state().selection().head();
    self.replace_session_doc(pm_doc, TransactionOrigin::History, Some(prev_pos));
    let state = self.session.state();
    self.sync_signals(&state, true);
    self.last_edited.set(String::from(DEFAULT_STATUS_TEXT));
    self.notify_persist(markdown, crdt_update);
  }

  fn insert_block_node(&self, node: Node, place_cursor_after: bool) {
    let state = self.session.state();
    let from = state.selection().resolved_from();
    let (insert_pos, replace_empty) = if from.depth >= 1 {
      let top = from.node(1);
      if top.ty().name() == "paragraph" && top.content().size() == 0 {
        (from.before(1), Some((from.before(1), from.after(1))))
      } else {
        (from.after(1), None)
      }
    } else {
      (state.doc().content().size(), None)
    };
    let node_size = node.node_size();
    let end_after = replace_empty.map_or(insert_pos, |(_, e)| e);
    let mut nodes = vec![node];
    if place_cursor_after
      && end_after >= state.doc().content().size()
      && let Ok(para) = self.schema.node("paragraph", Some(make_attrs(new_block_id(), &[])), Fragment::empty(), Vec::new())
    {
      nodes.push(para);
    }
    let mut tr = state.tr();
    let res = match replace_empty {
      Some((s, e)) => tr.replace_with(s, e, Fragment::from(nodes)),
      None => tr.insert(insert_pos, Fragment::from(nodes)),
    };
    if res.is_err() {
      return;
    }
    let target = if place_cursor_after { insert_pos + node_size } else { insert_pos + 1 };
    if let Ok(r) = tr.doc().resolve(target.min(tr.doc().content().size())) {
      tr.set_selection(Selection::near(&r, 1));
    }
    tr.scroll_into_view();
    let _ = self.session.dispatch(tr);
  }

  fn replace_session_doc(&self, new_doc: Node, origin: TransactionOrigin, preserve_pos: Option<usize>) {
    let state = self.session.state();
    let mut tr = state.tr();
    let old_size = tr.doc().content().size();
    let _ = tr.replace_with(0, old_size, new_doc.content().clone());
    let target = preserve_pos.map(|p| p.min(tr.doc().content().size())).unwrap_or(0);
    let sel = tr.doc().resolve(target).map(|r| Selection::near(&r, 1)).unwrap_or_else(|_| Selection::at_start(tr.doc()));
    tr.set_selection(sel);
    tr.set_origin(origin);
    tr.set_meta(SKIP_TRAILING_NODE, Value::Bool(true));
    tr.set_meta(MICA_SYNC_ORIGIN_META, Value::Bool(true));
    let _ = self.session.dispatch(tr);
  }

  fn handle_dispatch(&self, session: &EditorSession, tr: Transaction) {
    let skip_crdt = matches!(tr.origin(), Some(TransactionOrigin::Remote(_) | TransactionOrigin::History))
      || tr.get_meta(MICA_SYNC_ORIGIN_META).is_some();

    let old_state = session.state();
    let Ok((mut next_state, mut applied)) = old_state.apply_transaction(tr) else { return };
    if applied.is_empty() {
      return;
    }
    let doc_changed = !old_state.doc().eq(next_state.doc());
    if !skip_crdt
      && doc_changed
      && let Some(id_tr) = ensure_unique_block_ids(&next_state)
      && let Ok((fixed, extra)) = next_state.apply_transaction(id_tr)
    {
      next_state = fixed;
      applied.extend(extra);
    }
    session.commit(next_state.clone(), &applied);

    if !skip_crdt && doc_changed {
      let next_blocks = blocks_from_pm_doc(next_state.doc());
      let payload = {
        let mut prev = self.prev_blocks.borrow_mut();
        if *prev == next_blocks {
          None
        } else {
          let root_id = self.root_id.borrow().clone();
          let mut doc = self.doc.borrow_mut();
          let before_sv = doc.state_vector();
          doc.begin_history_batch();
          apply_block_diff(&mut doc, &root_id, &prev, &next_blocks);
          doc.end_history_batch();
          let diff = doc.encode_diff(&before_sv).ok().filter(|u| !u.is_empty());
          let md = serialize_doc_markdown(&doc);
          *prev = next_blocks;
          Some((md, diff))
        }
      };
      self.sync_signals(&next_state, true);
      if let Some((markdown, crdt_update)) = payload {
        self.last_edited.set(String::from(DEFAULT_STATUS_TEXT));
        self.notify_persist(markdown, crdt_update);
      }
    } else {
      self.sync_signals(&next_state, doc_changed);
    }
  }

  fn notify_persist(&self, markdown: String, crdt_update: Option<Vec<u8>>) {
    if let Some(cb) = self.on_persist.borrow().clone() {
      cb(markdown, crdt_update);
    }
  }

  fn sync_signals(&self, state: &EditorState, doc_changed: bool) {
    if doc_changed {
      self.word_count.set(compute_word_count_label(state.doc()));
      self.block_count.set(state.doc().child_count());
      self.outline.set(compute_outline(state.doc()));
    }
    let (can_u, can_r) = {
      let doc = self.doc.borrow();
      (doc.can_undo(), doc.can_redo())
    };
    set_if_changed(self.can_undo, can_u);
    set_if_changed(self.can_redo, can_r);
    let (kind, level, marks) = compute_active_state(state);
    set_if_changed(self.active_kind, kind);
    set_if_changed(self.active_level, level);
    set_if_changed(self.active_marks, marks);
  }
}

// ── 7. Unit Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
  use std::{cell::RefCell, rc::Rc};

  use nsh_base::rich_editor::{
    extensions::history::history_key,
    state::{KeyEvent, TextSelection},
  };
  use nsh_core::element::reactive::Runtime;

  use super::*;

  fn with_runtime<R>(f: impl FnOnce() -> R) -> R {
    let rt = Runtime::default();
    let _entered = rt.enter();
    f()
  }

  fn type_text(session: &EditorSession, text: &str) {
    for ch in text.chars() {
      let s = ch.to_string();
      let state = session.state();
      let (from, to) = (state.selection().from(), state.selection().to());
      let mut out = None;
      let handled = state.plugins().iter().any(|p| p.handle_text_input(&state, from, to, &s, &mut |tr| out = Some(tr)));
      if handled {
        if let Some(tr) = out {
          session.dispatch(tr).expect("input rule tx dispatches");
        }
      } else {
        let mut tr = state.tr();
        tr.insert_text(&s, Some(from), Some(to));
        session.dispatch(tr).expect("text insert tx dispatches");
      }
    }
  }

  fn press_key(session: &EditorSession, event: KeyEvent) -> bool {
    let state = session.state();
    let mut trs = Vec::new();
    let handled = state.plugins().iter().any(|p| p.handle_key_down(&state, &event, &mut |tr| trs.push(tr)));
    for tr in trs {
      session.dispatch(tr).expect("key tx dispatches");
    }
    handled
  }

  #[test]
  fn schema_is_valid_and_has_expected_nodes_and_marks() {
    let schema = mica_schema();
    assert_eq!(schema.top_node_type().name(), "doc");
    assert_eq!(
      schema.top_node_type().content_match().default_type().as_ref().map(|t| t.name()),
      Some("paragraph")
    );
    for name in [
      "doc", "paragraph", "heading", "bulleted_list", "numbered_list", "todo", "quote",
      "code_block", "divider", "math", "mermaid", "board", "image", "table", "tableRow",
      "tableHeader", "tableCell", "text",
    ] {
      assert!(schema.node_type(name).is_some(), "missing node {name}");
    }
    for mark in MARK_NAMES {
      assert!(schema.mark_type(mark).is_some(), "missing mark {mark}");
    }
    assert!(tables::table_node_types(&schema).is_some());
  }

  #[test]
  fn markdown_rich_editor_mica_doc_round_trip() {
    with_runtime(|| {
      let md = "\
# Main Title

## Sub Section

A paragraph with **bold**, *italic*, `code`, ~~strike~~, and [Mica](https://mica.dev).

- Bullet one
- Bullet two

1. First item
2. Second item

- [ ] Open task
- [x] Done task

> A quoted line

```rust
fn hello() {}
```

---

| Name | Value |
| --- | --- |
| Alpha | **42** |
| Beta | `ok` |
";
      let bridge = MicaDocumentBridge::from_markdown(md);
      let state = bridge.session().state();
      assert!(!state.plugins().iter().any(|p| p.key() == history_key()));

      let kinds: Vec<&str> = state.doc().content().iter().map(|n| n.ty().name()).collect();
      for expected in ["heading", "paragraph", "bulleted_list", "numbered_list", "todo", "quote", "code_block", "divider", "table"] {
        assert!(kinds.contains(&expected), "missing {expected} in {kinds:?}");
      }

      let outline = bridge.outline_signal().get_untracked();
      assert_eq!(outline.len(), 2);
      assert_eq!((outline[0].level, outline[0].text.as_str()), (1, "Main Title"));
      assert_eq!((outline[1].level, outline[1].text.as_str()), (2, "Sub Section"));

      let projected = blocks_from_pm_doc(state.doc());
      let mut root = Block::new("root", "page");
      root.children = projected.iter().map(|b| b.id.clone()).collect();
      let mut all_blocks = vec![root];
      all_blocks.extend(projected);
      let projected_md = serialize_doc_markdown(&MicaDoc::from_blocks("root", &all_blocks));

      for exported in [bridge.export_markdown(), projected_md] {
        for snippet in [
          "# Main Title", "## Sub Section", "**bold**", "*italic*", "`code`", "~~strike~~",
          "[Mica](https://mica.dev)", "- Bullet one", "1. First item", "- [ ] Open task",
          "- [x] Done task", "> A quoted line", "```rust\nfn hello() {}\n```", "---",
          "| Name | Value |", "| Alpha | **42** |",
        ] {
          assert!(exported.contains(snippet), "missing `{snippet}` in:\n{exported}");
        }
      }
      let html = bridge.export_html();
      assert!(html.contains("Main Title") && html.contains("<table"));
    });
  }

  #[test]
  fn incremental_edit_sync_and_remote_convergence() {
    with_runtime(|| {
      let bridge_a = MicaDocumentBridge::from_markdown("# Title\n\nSecond paragraph");
      let initial_state = bridge_a.doc.borrow().encode_state();
      let bridge_b = MicaDocumentBridge::from_update(&initial_state).expect("peer B seeds from CRDT");

      type PersistLog = Rc<RefCell<Vec<(String, Option<Vec<u8>>)>>>;
      let persisted: PersistLog = Rc::new(RefCell::new(Vec::new()));
      let p_clone = persisted.clone();
      bridge_a.set_on_persist(Rc::new(move |md, upd| p_clone.borrow_mut().push((md, upd))));

      let session = bridge_a.session();
      let heading_id_before = attr_str(session.state().doc().child(0).attrs(), "id").to_owned();
      let doc = session.state().doc().clone();
      let mut sel_tr = session.state().tr();
      sel_tr.set_selection(TextSelection::create(&doc, doc.content().size() - 1, None));
      session.dispatch(sel_tr).unwrap();

      type_text(&session, " edited");
      assert_eq!(heading_id_before, attr_str(session.state().doc().child(0).attrs(), "id"));
      let events = persisted.borrow();
      assert!(!events.is_empty());
      for (_, upd) in events.iter() {
        if let Some(bytes) = upd {
          bridge_b.apply_remote_update(bytes).expect("remote update converges");
        }
      }
      assert_eq!(bridge_a.export_markdown(), bridge_b.export_markdown());
      assert_eq!(bridge_b.session().state().doc().child(1).text_content(), "Second paragraph edited");
    });
  }

  #[test]
  fn single_stack_crdt_undo_redo_via_api_and_keymap() {
    with_runtime(|| {
      let bridge = MicaDocumentBridge::from_markdown("Hello");
      let session = bridge.session();
      assert!(!bridge.can_undo_signal().get_untracked());
      assert!(!bridge.can_redo_signal().get_untracked());

      let doc = session.state().doc().clone();
      let mut sel_tr = session.state().tr();
      sel_tr.set_selection(TextSelection::create(&doc, 6, None));
      session.dispatch(sel_tr).unwrap();

      let mut tr = session.state().tr();
      tr.insert_text(" World", Some(6), Some(6));
      session.dispatch(tr).unwrap();
      assert_eq!(bridge.export_markdown().trim(), "Hello World");
      assert!(bridge.can_undo_signal().get_untracked());

      let mut mod_z = KeyEvent::new("z");
      mod_z.ctrl = true;
      mod_z.mod_key = true;
      assert!(press_key(&session, mod_z));
      assert_eq!(bridge.export_markdown().trim(), "Hello");
      assert!(!bridge.can_undo_signal().get_untracked());
      assert!(bridge.can_redo_signal().get_untracked());

      let mut shift_mod_z = KeyEvent::new("z");
      shift_mod_z.ctrl = true;
      shift_mod_z.mod_key = true;
      shift_mod_z.shift = true;
      assert!(press_key(&session, shift_mod_z));
      assert_eq!(bridge.export_markdown().trim(), "Hello World");

      bridge.undo();
      assert_eq!(bridge.export_markdown().trim(), "Hello");
      bridge.redo();
      assert_eq!(bridge.export_markdown().trim(), "Hello World");
    });
  }

  #[test]
  fn input_rules_and_flat_block_editing_keys() {
    with_runtime(|| {
      let bridge = MicaDocumentBridge::from_markdown("");
      let session = bridge.session();

      type_text(&session, "## Title");
      assert_eq!(session.state().doc().child(0).ty().name(), "heading");
      assert_eq!(attr_u64(session.state().doc().child(0).attrs(), "level", 0), 2);
      assert_eq!(bridge.active_kind_signal().get_untracked(), "heading");
      assert_eq!(bridge.active_level_signal().get_untracked(), 2);

      assert!(press_key(&session, KeyEvent::new("Enter")));
      assert_eq!(session.state().doc().child_count(), 2);
      assert_eq!(session.state().doc().child(1).ty().name(), "paragraph");
      assert_ne!(
        attr_str(session.state().doc().child(0).attrs(), "id"),
        attr_str(session.state().doc().child(1).attrs(), "id")
      );

      type_text(&session, "- item one");
      assert_eq!(session.state().doc().child(1).ty().name(), "bulleted_list");
      assert!(press_key(&session, KeyEvent::new("Enter")));
      assert_eq!(session.state().doc().child(2).ty().name(), "bulleted_list");
      assert!(press_key(&session, KeyEvent::new("Tab")));
      assert_eq!(attr_u64(session.state().doc().child(2).attrs(), "indent", 0), 1);

      let mut shift_tab = KeyEvent::new("Tab");
      shift_tab.shift = true;
      assert!(press_key(&session, shift_tab));
      assert_eq!(attr_u64(session.state().doc().child(2).attrs(), "indent", 99), 0);

      assert!(press_key(&session, KeyEvent::new("Backspace")));
      assert_eq!(session.state().doc().child(2).ty().name(), "paragraph");

      bridge.insert_divider();
      bridge.insert_table(2, 2);
      bridge.insert_custom_block("math", r"E = mc^2", None);
      let md = bridge.export_markdown();
      assert!(md.contains("## Title") && md.contains("- item one") && md.contains("---") && md.contains("E = mc^2"));
    });
  }
}
