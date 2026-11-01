//! Pasted clipboard HTML → Markdown, which the caller then parses as a
//! fragment.
//!
//! This used to be a Dart file (`lib/editor/html_to_markdown.dart`, removed
//! 2026-11-01): the last place outside this crate that WROTE Markdown syntax —
//! escaping, emphasis markers, list indentation — and so had to agree with the
//! parser on what text re-parses as what, by hand. It now sits beside that
//! parser. The client still tokenizes the HTML (with the `html` package it
//! already ships, so this crate stays free of an HTML parser dependency) and
//! hands over a neutral element tree, [`HtmlNode`].
//!
//! Fidelity contract: what the source page SHOWED is what lands in the
//! document. Text-node content is backslash-escaped so literal `*`/`#`/`>`/…
//! cannot re-parse as structure; markers this converter emits itself (`- `,
//! `# `, `> `, `**`) are added after escaping and stay live.
//!
//! The behaviour is a line-for-line port of the Dart converter; its many
//! client tests (paste structure, inline marks, Word lists, math, clipboard
//! round trips) now run through this code.

use std::collections::BTreeMap;

use serde_json::Value;

/// One node of a parsed HTML tree, as the client hands it over.
///
/// `{"t": text}` is a text node (entities already decoded), `{"e": tag, "a":
/// {attr: value}, "c": [children]}` an element (tag and attribute names
/// lower-case), and `{"m": …}` anything else (a comment) — kept because, like
/// any non-inline node, it ends a run of inline siblings.
#[derive(Debug, Clone, PartialEq)]
pub enum HtmlNode {
  /// A text node.
  Text {
    /// The decoded text.
    t: String,
  },
  /// An element.
  Element {
    /// Lower-case tag name.
    e: String,
    /// Attributes by lower-case name.
    a: BTreeMap<String, String>,
    /// Child nodes in document order.
    c: Vec<HtmlNode>,
  },
  /// A comment or other non-element node.
  Other,
}

impl HtmlNode {
  /// Read the client's JSON shape. Hand-written over [`Value`] rather than a
  /// derived untagged deserializer: the wasm build already carries `Value`,
  /// and the derive cost ~20 KB of a bundle with a hard 256 KiB budget.
  /// Anything that is neither a text node nor an element (including
  /// malformed entries) reads as [`HtmlNode::Other`].
  pub fn from_json(value: &Value) -> Self {
    let Some(obj) = value.as_object() else {
      return Self::Other;
    };
    if let Some(text) = obj.get("t").and_then(Value::as_str) {
      return Self::Text {
        t: text.to_string(),
      };
    }
    let Some(tag) = obj.get("e").and_then(Value::as_str) else {
      return Self::Other;
    };
    let a = obj
      .get("a")
      .and_then(Value::as_object)
      .map(|attrs| {
        attrs
          .iter()
          .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
          .collect()
      })
      .unwrap_or_default();
    let c = obj
      .get("c")
      .and_then(Value::as_array)
      .map(|children| children.iter().map(Self::from_json).collect())
      .unwrap_or_default();
    Self::Element {
      e: tag.to_string(),
      a,
      c,
    }
  }

  fn tag(&self) -> &str {
    match self {
      Self::Element { e, .. } => e,
      _ => "",
    }
  }
  fn attr(&self, name: &str) -> Option<&str> {
    match self {
      Self::Element { a, .. } => a.get(name).map(String::as_str),
      _ => None,
    }
  }
  fn has_attr(&self, name: &str) -> bool {
    self.attr(name).is_some()
  }
  fn nodes(&self) -> &[HtmlNode] {
    match self {
      Self::Element { c, .. } => c,
      _ => &[],
    }
  }
  fn is_element(&self) -> bool {
    matches!(self, Self::Element { .. })
  }
  /// Element children only.
  fn children(&self) -> impl Iterator<Item = &HtmlNode> {
    self.nodes().iter().filter(|n| n.is_element())
  }
  /// Concatenated descendant text (the DOM's `textContent`).
  fn text(&self) -> String {
    let mut out = String::new();
    self.collect_text(&mut out);
    out
  }
  fn collect_text(&self, out: &mut String) {
    match self {
      Self::Text { t } => out.push_str(t),
      Self::Element { c, .. } => c.iter().for_each(|n| n.collect_text(out)),
      Self::Other => {}
    }
  }
  fn has_class(&self, class: &str) -> bool {
    self
      .attr("class")
      .is_some_and(|c| c.split_ascii_whitespace().any(|x| x == class))
  }
  /// First descendant element (not self) whose tag is in `tags`.
  fn find_descendant(&self, tags: &[&str]) -> Option<&HtmlNode> {
    for child in self.children() {
      if tags.contains(&child.tag()) {
        return Some(child);
      }
      if let Some(found) = child.find_descendant(tags) {
        return Some(found);
      }
    }
    None
  }
  fn descendants<'a>(&'a self, out: &mut Vec<&'a HtmlNode>) {
    for child in self.children() {
      out.push(child);
      child.descendants(out);
    }
  }
}

/// Convert a parsed `<body>` (its child nodes are the fragment) to Markdown.
pub fn html_to_markdown(body: &HtmlNode) -> String {
  let mut c = Converter::default();
  let mut out = String::new();
  c.emit_children(body.nodes(), &mut out);
  collapse_blank_runs(&out)
    .trim_matches(is_trim_ws)
    .to_string()
}

#[derive(Default)]
struct Converter {
  /// Inside a `<blockquote>`: italic marks are dropped. Sources habitually
  /// STYLE quote text italic (Word quote styles, WeChat inline css, many site
  /// themes); that is presentation, not emphasis, and Mica renders quotes
  /// upright.
  no_italic: bool,
  /// Inside a `<blockquote>`: nested-list indentation is capped below 4
  /// columns, because the quote-side parser has no list stack and 4+ leading
  /// columns inside a quote become an indented CODE block.
  in_quote: bool,
  /// Word/WPS flat-list run state, see [`Converter::mso_list_item`].
  mso_indents: Option<Vec<String>>,
  mso_ordered: Vec<bool>,
  mso_base: i64,
}

// ── whitespace, matching the Dart semantics the converter was written in ──

/// JavaScript / Dart RegExp `\s`.
fn is_re_space(c: char) -> bool {
  matches!(
    c,
    '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
      ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
  )
}

/// Dart `String.trim()` (the RegExp set plus U+0085).
fn is_trim_ws(c: char) -> bool {
  c == '\u{85}' || is_re_space(c)
}

/// `replaceAll(RegExp(r'\s+'), ' ')`.
fn collapse_ws(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut in_run = false;
  for c in s.chars() {
    if is_re_space(c) {
      if !in_run {
        out.push(' ');
        in_run = true;
      }
    } else {
      out.push(c);
      in_run = false;
    }
  }
  out
}

fn collapse_trim(s: &str) -> String {
  collapse_ws(s).trim_matches(is_trim_ws).to_string()
}

/// `replaceAll(RegExp(r'\n{3,}'), '\n\n')`.
fn collapse_blank_runs(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut run = 0usize;
  for c in s.chars() {
    if c == '\n' {
      run += 1;
      if run <= 2 {
        out.push('\n');
      }
    } else {
      run = 0;
      out.push(c);
    }
  }
  out
}

fn writeln(out: &mut String, line: &str) {
  out.push_str(line);
  out.push('\n');
}

// ── escaping ──────────────────────────────────────────────────────────────

/// Backslash-escape characters that re-parse as INLINE Markdown (emphasis,
/// code spans, strikethrough, link brackets, raw HTML `<`, inline math `$`) —
/// EXCEPT valid math runs, which pass through verbatim, so a formula copied
/// from a browser (`$\eta = 2$`) becomes a math mark exactly as the same text
/// pasted as plain text would. A lone `$` (prices) still escapes.
fn escape_md_inline(s: &str) -> String {
  let spans = crate::parse::inline::math_run_spans(s);
  if spans.is_empty() {
    return escape_all_md_inline(s);
  }
  let chars: Vec<char> = s.chars().collect();
  let mut out = String::with_capacity(s.len());
  let mut cursor = 0usize;
  for (start, end) in spans {
    out.push_str(&escape_all_md_inline(
      &chars[cursor..start].iter().collect::<String>(),
    ));
    out.extend(&chars[start..end]);
    cursor = end;
  }
  out.push_str(&escape_all_md_inline(
    &chars[cursor..].iter().collect::<String>(),
  ));
  out
}

fn escape_all_md_inline(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for c in s.chars() {
    if matches!(c, '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '<' | '$') {
      out.push('\\');
    }
    out.push(c);
  }
  out
}

/// Escape a paragraph-level LINE that would re-parse as a block construct:
/// ATX headings, quotes, `-`/`+` bullets, ordered markers, thematic breaks.
fn escape_block_start(line: &str) -> String {
  let bytes = line.as_bytes();
  // ^(\d{1,9})([.)])(\s|$)
  let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
  if (1..=9).contains(&digits)
    && matches!(bytes.get(digits), Some(b'.' | b')'))
    && line[digits + 1..].chars().next().is_none_or(is_re_space)
  {
    return format!("{}\\{}", &line[..digits], &line[digits..]);
  }
  // ^(?:#{1,6}|[-+])(?:\s|$)
  let hashes = bytes.iter().take_while(|b| **b == b'#').count();
  let marker_len = if (1..=6).contains(&hashes) {
    Some(hashes)
  } else if matches!(bytes.first(), Some(b'-' | b'+')) {
    Some(1)
  } else {
    None
  };
  let marker_hit = marker_len.is_some_and(|n| line[n..].chars().next().is_none_or(is_re_space));
  // ^-{3,}\s*$
  let dashes = bytes.iter().take_while(|b| **b == b'-').count();
  let rule = dashes >= 3 && line[dashes..].chars().all(is_re_space);
  if marker_hit || line.starts_with('>') || rule {
    return format!("\\{line}");
  }
  line.to_string()
}

/// A link/image destination: `<>`-wrap when it contains whitespace or parens
/// (an unbalanced `)` would otherwise cut the destination short on re-parse).
fn wrap_dest(dest: &str) -> String {
  if dest.chars().any(|c| is_re_space(c) || c == '(' || c == ')') {
    format!("<{dest}>")
  } else {
    dest.to_string()
  }
}

fn clean_href(href: Option<&str>) -> Option<String> {
  let h = href.unwrap_or("").trim_matches(is_trim_ws);
  if h.is_empty() || h.starts_with('#') || h.starts_with("javascript:") {
    return None;
  }
  Some(h.to_string())
}

fn clean_src(src: Option<&str>) -> Option<String> {
  let s = src.unwrap_or("").trim_matches(is_trim_ws);
  if s.is_empty() || s.starts_with("data:") {
    return None;
  }
  Some(s.to_string())
}

fn image(alt: &str, src: &str) -> String {
  format!("![{}]({})", escape_md_inline(alt), wrap_dest(src))
}

/// Wrap inline content in emphasis markers, keeping leading/trailing
/// whitespace OUTSIDE the markers — CommonMark rejects `** x **`.
fn wrap_marks(inner: &str, bold: bool, italic: bool, strike: bool) -> String {
  let core = inner.trim_matches(is_trim_ws);
  if core.is_empty() {
    return inner.to_string();
  }
  let lead = &inner[..inner.len() - inner.trim_start_matches(is_trim_ws).len()];
  let trail = &inner[inner.trim_end_matches(is_trim_ws).len()..];
  let (mut open, mut close) = (String::new(), String::new());
  for (on, marker) in [(bold, "**"), (italic, "*"), (strike, "~~")] {
    if on {
      open.push_str(marker);
      close.insert_str(0, marker);
    }
  }
  format!("{lead}{open}{core}{close}{trail}")
}

/// Inline `<code>` → a backtick span: fence with one more backtick than the
/// longest run inside, padding a space when the content touches a backtick.
fn inline_code(raw: &str) -> String {
  let core = raw.trim_matches(is_trim_ws);
  if core.is_empty() {
    return raw.to_string();
  }
  let lead = &raw[..raw.len() - raw.trim_start_matches(is_trim_ws).len()];
  let trail = &raw[raw.trim_end_matches(is_trim_ws).len()..];
  let (mut max_run, mut cur) = (0usize, 0usize);
  for c in core.chars() {
    if c == '`' {
      cur += 1;
      max_run = max_run.max(cur);
    } else {
      cur = 0;
    }
  }
  let fence = "`".repeat(max_run + 1);
  let pad = if core.starts_with('`') || core.ends_with('`') {
    " "
  } else {
    ""
  };
  format!("{lead}{fence}{pad}{core}{pad}{fence}{trail}")
}

/// Inline elements (whitelist). Anything else is treated as block-level.
const INLINE_TAGS: &[&str] = &[
  "a", "strong", "b", "em", "i", "code", "s", "del", "strike", "u", "mark", "sub", "sup", "small",
  "span", "font", "br", "wbr", "abbr", "cite", "q", "kbd", "samp", "var", "time", "label", "math",
  "ins", "big", "tt",
];

fn is_inline_node(n: &HtmlNode) -> bool {
  match n {
    HtmlNode::Text { .. } => true,
    HtmlNode::Element { e, .. } => INLINE_TAGS.contains(&e.as_str()),
    HtmlNode::Other => false,
  }
}

/// Only formula roots carry source TeX: Gemini's `data-math`, and KaTeX /
/// MathML's `application/x-tex` annotation. Never a paragraph that merely
/// contains a formula (that would swallow the prose around it).
fn rendered_math_source(e: &HtmlNode) -> Option<String> {
  if e.has_class("math-inline")
    && let Some(source) = e.attr("data-math").map(|s| s.trim_matches(is_trim_ws))
    && !source.is_empty()
  {
    return Some(source.to_string());
  }
  if !e.has_class("katex") && e.tag() != "math" {
    return None;
  }
  let mut all = Vec::new();
  e.descendants(&mut all);
  for annotation in all.into_iter().filter(|n| n.tag() == "annotation") {
    if !annotation
      .attr("encoding")
      .is_some_and(|enc| enc.eq_ignore_ascii_case("application/x-tex"))
    {
      continue;
    }
    let source = annotation.text();
    let source = source.trim_matches(is_trim_ws);
    if !source.is_empty() {
      return Some(source.to_string());
    }
  }
  None
}

/// Bold / italic / strike implied by an element's tag AND its inline style —
/// Google Docs / Word emit styled `<span>`s (`font-weight:700`), not `<b>`.
fn inline_marks(e: &HtmlNode, no_italic: bool) -> (bool, bool, bool) {
  let (mut bold, mut italic, mut strike) = match e.tag() {
    "b" | "strong" => (true, false, false),
    "i" | "em" => (false, true, false),
    "s" | "del" | "strike" => (false, false, true),
    _ => (false, false, false),
  };
  let style = e.attr("style").unwrap_or("").to_lowercase();
  if let Some(weight) = style_value(&style, "font-weight") {
    let w: String = weight
      .chars()
      .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
      .collect();
    if w == "bold" || w == "bolder" || w.parse::<i64>().unwrap_or(0) >= 600 {
      bold = true;
    }
  }
  if style_value(&style, "font-style").is_some_and(|v| v.starts_with("italic")) {
    italic = true;
  }
  if let Some(pos) = style.find("text-decoration") {
    let rest = &style[pos..];
    let decl = &rest[..rest.find(';').unwrap_or(rest.len())];
    if decl.contains("line-through") {
      strike = true;
    }
  }
  if no_italic {
    italic = false;
  }
  (bold, italic, strike)
}

/// The value after `name\s*:\s*` at the first occurrence of `name` that has
/// one (RegExp `name\s*:\s*(…)` semantics: later occurrences are tried too).
fn style_value<'a>(style: &'a str, name: &str) -> Option<&'a str> {
  let mut from = 0;
  while let Some(pos) = style[from..].find(name) {
    let after = &style[from + pos + name.len()..];
    let after = after.trim_start_matches(is_re_space);
    if let Some(rest) = after.strip_prefix(':') {
      return Some(rest.trim_start_matches(is_re_space));
    }
    from += pos + name.len();
  }
  None
}

/// `mso-list:[^;"]*\blevel(\d+)` (case-insensitive) → the level digits.
fn mso_level(style: &str) -> Option<String> {
  let lower = style.to_lowercase();
  let mut from = 0;
  while let Some(pos) = lower[from..].find("mso-list:") {
    let start = from + pos + "mso-list:".len();
    let end = lower[start..]
      .find([';', '"'])
      .map_or(lower.len(), |e| start + e);
    let decl = &lower[start..end];
    let mut search = 0;
    while let Some(lp) = decl[search..].find("level") {
      let at = search + lp;
      let boundary = at == 0
        || !decl[..at]
          .chars()
          .next_back()
          .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
      let digits: String = decl[at + 5..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
      if boundary && !digits.is_empty() {
        return Some(digits);
      }
      search = at + 5;
    }
    from = end.max(start);
  }
  None
}

fn is_mso_list_p(e: Option<&HtmlNode>) -> bool {
  e.is_some_and(|e| e.tag() == "p" && mso_level(e.attr("style").unwrap_or("")).is_some())
}

/// `^\(?([0-9a-zA-Z]{1,3}|[一二三四五六七八九十]{1,3})[.)、]`. (Not consuming
/// a leading `(` can never help: neither class matches it.)
fn is_ordered_marker(marker: &str) -> bool {
  let chars: Vec<char> = marker.chars().collect();
  let start = usize::from(chars.first() == Some(&'('));
  let closer = |j: usize| chars.get(j).is_some_and(|c| matches!(c, '.' | ')' | '、'));
  let ascii = |c: char| c.is_ascii_alphanumeric();
  let cjk = |c: char| "一二三四五六七八九十".contains(c);
  [&ascii as &dyn Fn(char) -> bool, &cjk]
    .into_iter()
    .any(|class| {
      // Every end position after 1–3 matching chars (regex backtracking).
      let mut j = start;
      while j < chars.len() && j - start < 3 && class(chars[j]) {
        j += 1;
        if closer(j) {
          return true;
        }
      }
      false
    })
}

/// Remove the first `<span>` whose style marks it `mso-list:Ignore` (Word's
/// bullet/number glyph), returning its trimmed text.
fn take_mso_marker(node: &mut HtmlNode) -> Option<String> {
  let HtmlNode::Element { c, .. } = node else {
    return None;
  };
  for idx in 0..c.len() {
    let is_marker = c[idx].tag() == "span" && {
      let st = c[idx].attr("style").unwrap_or("").to_lowercase();
      st.contains("mso-list") && st.contains("ignore")
    };
    if is_marker {
      let text = c[idx].text();
      c.remove(idx);
      return Some(text.trim_matches(is_trim_ws).to_string());
    }
    if let Some(found) = take_mso_marker(&mut c[idx]) {
      return Some(found);
    }
  }
  None
}

/// The GFM task-list checkbox of a list item: the first `<input
/// type=checkbox>` whose nearest enclosing `<li>` is the item itself.
fn task_checkbox(item: &HtmlNode) -> Option<&HtmlNode> {
  fn walk(n: &HtmlNode) -> Option<&HtmlNode> {
    for child in n.children() {
      if child.tag() == "input"
        && child
          .attr("type")
          .unwrap_or("")
          .eq_ignore_ascii_case("checkbox")
      {
        return Some(child);
      }
      if child.tag() == "li" {
        continue; // its inputs belong to that item
      }
      if let Some(found) = walk(child) {
        return Some(found);
      }
    }
    None
  }
  walk(item)
}

/// A list's items and nested lists, seen THROUGH wrapper elements (Google's
/// AI overview puts `<div>`s between `<ol>` and `<li>`).
fn list_items<'a>(list: &'a HtmlNode, out: &mut Vec<&'a HtmlNode>) {
  for child in list.children() {
    if matches!(child.tag(), "li" | "ul" | "ol") {
      out.push(child);
    } else {
      list_items(child, out);
    }
  }
}

fn code_text(node: &HtmlNode) -> String {
  fn append_newline(sb: &mut String) {
    if !sb.is_empty() && !sb.ends_with('\n') {
      sb.push('\n');
    }
  }
  fn gather(node: &HtmlNode, sb: &mut String) {
    for child in node.nodes() {
      match child {
        HtmlNode::Text { t } => sb.push_str(t),
        HtmlNode::Element { e, .. } => {
          if e == "br" {
            append_newline(sb);
          } else {
            gather(child, sb);
            if matches!(e.as_str(), "div" | "p" | "tr" | "li") {
              append_newline(sb);
            }
          }
        }
        HtmlNode::Other => {}
      }
    }
  }
  let mut sb = String::new();
  gather(node, &mut sb);
  sb
}

/// Dart `int.tryParse` on an attribute: optional sign, digits, surrounding
/// whitespace allowed.
fn parse_int(s: Option<&str>) -> Option<i64> {
  s?.trim_matches(is_trim_ws).parse::<i64>().ok()
}

impl Converter {
  /// Emit a node list, coalescing runs of consecutive inline siblings into
  /// ONE paragraph (a single paragraph copied from Typora arrives with no
  /// `<p>` wrapper).
  fn emit_children(&mut self, nodes: &[HtmlNode], out: &mut String) {
    let mut run: Vec<&HtmlNode> = Vec::new();
    let elements: Vec<usize> = (0..nodes.len())
      .filter(|&i| nodes[i].is_element())
      .collect();
    for (i, n) in nodes.iter().enumerate() {
      if is_inline_node(n) {
        run.push(n);
        continue;
      }
      self.flush_run(&mut run, out);
      let pos = elements.iter().position(|&e| e == i);
      let prev = pos
        .and_then(|p| p.checked_sub(1))
        .map(|p| &nodes[elements[p]]);
      let next = pos.and_then(|p| elements.get(p + 1)).map(|&e| &nodes[e]);
      self.node(n, prev, next, out);
    }
    self.flush_run(&mut run, out);
  }

  fn flush_run(&mut self, run: &mut Vec<&HtmlNode>, out: &mut String) {
    if run.is_empty() {
      return;
    }
    let mut sb = String::new();
    for n in run.drain(..) {
      self.gather_one(n, &mut sb);
    }
    let text = collapse_trim(&sb);
    if !text.is_empty() {
      writeln(out, &escape_block_start(&text));
      writeln(out, "");
    }
  }

  fn node(
    &mut self,
    node: &HtmlNode,
    prev: Option<&HtmlNode>,
    next: Option<&HtmlNode>,
    out: &mut String,
  ) {
    let HtmlNode::Element { e: tag, .. } = node else {
      if let HtmlNode::Text { t } = node {
        let text = collapse_trim(&escape_md_inline(t));
        if !text.is_empty() {
          writeln(out, &escape_block_start(&text));
          writeln(out, "");
        }
      }
      return;
    };
    match tag.as_str() {
      "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
        let level: usize = tag[1..].parse().unwrap_or(1);
        let text = self.inline(node);
        if !text.is_empty() {
          writeln(out, &format!("{} {text}", "#".repeat(level)));
          writeln(out, "");
        }
      }
      "p" => {
        // Mica's own copy flavor: a footnote definition travels as
        // `<p data-mica-fndef="label">` — rebuild the GFM leader.
        if let Some(label) = node.attr("data-mica-fndef") {
          let text = self.inline(node);
          writeln(out, &format!("[^{label}]: {text}"));
          writeln(out, "");
          return;
        }
        if self.mso_list_item(node, prev, next, out) {
          return;
        }
        let text = self.inline(node);
        if !text.is_empty() {
          writeln(out, &escape_block_start(&text));
          writeln(out, "");
        }
      }
      "pre" => {
        self.fenced_code(node, out, "");
        writeln(out, "");
      }
      "blockquote" => self.emit_quote(node, out, ""),
      "ul" | "ol" => {
        self.list(node, out, tag == "ol", "");
        writeln(out, "");
      }
      "table" => self.table(node, out),
      "hr" => {
        writeln(out, "---");
        writeln(out, "");
      }
      "br" => writeln(out, ""),
      "img" => {
        if let Some(src) = clean_src(node.attr("src")) {
          writeln(out, &image(node.attr("alt").unwrap_or(""), &src));
          writeln(out, "");
        }
      }
      "dl" => {
        // No definition lists in Markdown: a term becomes a bold line, a
        // definition its own paragraph.
        for child in node.children() {
          let ct = child.tag();
          if ct != "dt" && ct != "dd" {
            continue;
          }
          let text = self.inline(child);
          if text.is_empty() {
            continue;
          }
          let line = if ct == "dt" {
            format!("**{text}**")
          } else {
            escape_block_start(&text)
          };
          writeln(out, &line);
          writeln(out, "");
        }
      }
      "div" | "section" | "article" | "main" | "header" | "footer" | "aside" | "nav" | "figure"
      | "picture" | "span" => self.emit_children(node.nodes(), out),
      _ => {
        const BLOCKS: &[&str] = &[
          "table",
          "ul",
          "ol",
          "pre",
          "blockquote",
          "h1",
          "h2",
          "h3",
          "h4",
          "h5",
          "h6",
          "p",
          "img",
        ];
        if node.find_descendant(BLOCKS).is_some() {
          self.emit_children(node.nodes(), out);
          return;
        }
        let text = self.inline(node);
        if !text.is_empty() {
          writeln(out, &escape_block_start(&text));
          writeln(out, "");
        }
      }
    }
  }

  /// A `<blockquote>`'s children converted recursively, then every line
  /// prefixed with `> ` (a bare `>` on blanks keeps one quote group).
  fn emit_quote(&mut self, node: &HtmlNode, out: &mut String, indent: &str) {
    let mut inner = String::new();
    let (saved_italic, saved_quote) = (self.no_italic, self.in_quote);
    self.no_italic = true;
    self.in_quote = true;
    self.emit_children(node.nodes(), &mut inner);
    self.no_italic = saved_italic;
    self.in_quote = saved_quote;
    let body = collapse_blank_runs(&inner);
    let body = body.trim_end_matches(is_trim_ws);
    if body.is_empty() {
      return;
    }
    for line in body.split('\n') {
      if line.is_empty() {
        writeln(out, &format!("{indent}>"));
      } else {
        writeln(out, &format!("{indent}> {line}"));
      }
    }
    writeln(out, "");
  }

  /// `<pre>` → a fenced block at `indent`; the language comes from a
  /// `language-*` / `lang-*` class on `<pre>` or its `<code>`.
  fn fenced_code(&mut self, pre: &HtmlNode, out: &mut String, indent: &str) {
    let class = format!(
      "{} {}",
      pre.attr("class").unwrap_or(""),
      pre
        .find_descendant(&["code"])
        .and_then(|c| c.attr("class"))
        .unwrap_or("")
    );
    let lang = code_language(&class);
    writeln(out, &format!("{indent}```{lang}"));
    let text = code_text(pre);
    for line in text.trim_end_matches(is_trim_ws).split('\n') {
      writeln(out, &format!("{indent}{line}"));
    }
    writeln(out, &format!("{indent}```"));
  }

  fn cap_quote_indent(&self, indent: String) -> String {
    if self.in_quote && indent.len() >= 4 {
      "   ".to_string()
    } else {
      indent
    }
  }

  fn list(&mut self, list: &HtmlNode, out: &mut String, ordered: bool, indent: &str) {
    // `<ol start="5">` seeds the numbering.
    let mut index = parse_int(list.attr("start")).unwrap_or(1);
    // A list nested as a DIRECT child of a list (invalid, but Word emits it)
    // belongs under the last item written.
    let mut last_child_indent = self.cap_quote_indent(format!("{indent}  "));
    let mut items = Vec::new();
    list_items(list, &mut items);
    for item in items {
      let t = item.tag();
      if t == "ul" || t == "ol" {
        let child_indent = last_child_indent.clone();
        self.list(item, out, t == "ol", &child_indent);
        continue;
      }
      if t != "li" {
        continue;
      }
      let checkbox = task_checkbox(item);
      let marker = match checkbox {
        Some(b) if b.has_attr("checked") => "- [x]".to_string(),
        Some(_) => "- [ ]".to_string(),
        None if ordered => format!("{index}."),
        None => "-".to_string(),
      };
      let text = self.direct_inline(item);
      writeln(out, &format!("{indent}{marker} {text}"));
      // Children indent to THIS item's content column (`12. ` is 4 wide);
      // for todos only `- ` is the marker.
      let width = if checkbox.is_some() {
        2
      } else {
        marker.chars().count() + 1
      };
      let child_indent = self.cap_quote_indent(format!("{indent}{}", " ".repeat(width)));
      last_child_indent = child_indent.clone();
      for child in item.children() {
        match child.tag() {
          "ul" | "ol" => self.list(child, out, child.tag() == "ol", &child_indent),
          "pre" => self.fenced_code(child, out, &child_indent),
          "blockquote" => self.emit_quote(child, out, &child_indent),
          "table" => {
            let mut sb = String::new();
            self.table(child, &mut sb);
            for line in sb.trim_end_matches(is_trim_ws).split('\n') {
              writeln(out, &format!("{child_indent}{line}"));
            }
          }
          _ => {}
        }
      }
      index += 1;
    }
  }

  /// Word/WPS "flat" list paragraphs: each item is a `<p
  /// style='…mso-list:l0 level2 lfo1'>` whose glyph sits in a
  /// `mso-list:Ignore` span. Levels are ABSOLUTE, so indentation is emitted
  /// relative to the run's first item. False when not a Word list item.
  fn mso_list_item(
    &mut self,
    p: &HtmlNode,
    prev: Option<&HtmlNode>,
    next: Option<&HtmlNode>,
    out: &mut String,
  ) -> bool {
    let Some(level) = mso_level(p.attr("style").unwrap_or("")) else {
      return false;
    };
    let level = level.parse::<i64>().unwrap_or(1).clamp(1, 9);
    let mut p = p.clone();
    let marker_text = take_mso_marker(&mut p).unwrap_or_default();
    let ordered = is_ordered_marker(&marker_text);

    if self.mso_indents.is_none() || !is_mso_list_p(prev) {
      self.mso_indents = None; // a new run
    }
    if self.mso_indents.is_none() || level < self.mso_base {
      self.mso_base = level;
      self.mso_indents = Some(vec![String::new()]);
      self.mso_ordered = vec![ordered];
    }
    let in_quote = self.in_quote;
    let indents = self.mso_indents.as_mut().expect("set above");
    let mut rel = (level - self.mso_base) as usize;
    if rel >= indents.len() {
      // Deeper: indent to the deepest known item's content column (level
      // jumps of 2+ clamp to one step).
      let parent_width = if *self.mso_ordered.last().unwrap_or(&false) {
        3
      } else {
        2
      };
      let next_indent = format!(
        "{}{}",
        indents.last().map(String::as_str).unwrap_or(""),
        " ".repeat(parent_width)
      );
      let next_indent = if in_quote && next_indent.len() >= 4 {
        "   ".to_string()
      } else {
        next_indent
      };
      indents.push(next_indent);
      self.mso_ordered.push(ordered);
      rel = indents.len() - 1;
    } else {
      self.mso_ordered[rel] = ordered;
      indents.truncate(rel + 1);
      self.mso_ordered.truncate(rel + 1);
    }
    let line_indent = indents[rel].clone();

    let text = self.inline(&p);
    if !text.is_empty() {
      let bullet = if ordered { "1." } else { "-" };
      writeln(out, &format!("{line_indent}{bullet} {text}"));
      // A blank only after the run's last item: between items it would push
      // a nested item over the indented-code threshold.
      if !is_mso_list_p(next) {
        writeln(out, "");
      }
    }
    true
  }

  fn table(&mut self, table: &HtmlNode, out: &mut String) {
    // Only THIS table's rows, not those of nested layout tables.
    fn collect<'a>(el: &'a HtmlNode, rows: &mut Vec<&'a HtmlNode>) {
      for c in el.children() {
        match c.tag() {
          "tr" => rows.push(c),
          "thead" | "tbody" | "tfoot" => collect(c, rows),
          _ => {}
        }
      }
    }
    let mut rows = Vec::new();
    collect(table, &mut rows);
    if rows.is_empty() {
      return;
    }
    let mut header_written = false;
    for row in rows {
      let cells: Vec<&HtmlNode> = row
        .children()
        .filter(|c| matches!(c.tag(), "td" | "th"))
        .collect();
      if cells.is_empty() {
        continue;
      }
      let texts: Vec<String> = cells
        .iter()
        .map(|c| self.inline(c).replace('|', r"\|").replace('\n', " "))
        .collect();
      writeln(out, &format!("| {} |", texts.join(" | ")));
      if !header_written {
        let sep: Vec<&str> = cells.iter().map(|_| "---").collect();
        writeln(out, &format!("| {} |", sep.join(" | ")));
        header_written = true;
      }
    }
    writeln(out, "");
  }

  fn inline(&mut self, node: &HtmlNode) -> String {
    let mut sb = String::new();
    self.gather(node, &mut sb);
    // `\s+`, not spaces: source-formatting newlines inside a heading or
    // emphasis are inter-word whitespace.
    collapse_trim(&sb)
  }

  /// An item's own inline content: its block children (lists, code, quotes,
  /// tables) are emitted as child blocks instead.
  fn direct_inline(&mut self, item: &HtmlNode) -> String {
    let mut sb = String::new();
    for child in item.nodes() {
      if child.is_element()
        && matches!(
          child.tag(),
          "ul" | "ol" | "pre" | "blockquote" | "table" | "input"
        )
      {
        continue;
      }
      self.gather_one(child, &mut sb);
    }
    collapse_trim(&sb)
  }

  fn gather(&mut self, node: &HtmlNode, sb: &mut String) {
    for child in node.nodes() {
      self.gather_one(child, sb);
    }
  }

  /// One inline node's Markdown appended to `sb`.
  fn gather_one(&mut self, child: &HtmlNode, sb: &mut String) {
    let tag = match child {
      HtmlNode::Text { t } => {
        let text = escape_md_inline(t);
        // A literal `!` right before an `<a>` would read back as an image.
        match text.strip_suffix('!') {
          Some(head) => {
            sb.push_str(head);
            sb.push_str("\\!");
          }
          None => sb.push_str(&text),
        }
        return;
      }
      HtmlNode::Element { e, .. } => e.as_str(),
      HtmlNode::Other => return,
    };
    // Mica's own copy flavor: formulas and footnote refs keep their source.
    if child.has_attr("data-mica-math") {
      sb.push_str(&child.text());
      return;
    }
    if let Some(source) = rendered_math_source(child) {
      // Paren delimiters also work beside digits and with `$` in the TeX.
      sb.push_str("\\(");
      sb.push_str(&source);
      sb.push_str("\\)");
      return;
    }
    if let Some(label) = child.attr("data-mica-footnote") {
      sb.push_str(&format!("[^{label}]"));
      return;
    }
    match tag {
      "br" => sb.push(' '),
      "a" => {
        let href = clean_href(child.attr("href"));
        let mut inner = String::new();
        self.gather(child, &mut inner);
        let text = inner.trim_matches(is_trim_ws);
        match href {
          Some(href) if !text.is_empty() => {
            sb.push_str(&format!("[{text}]({})", wrap_dest(&href)));
          }
          _ => sb.push_str(text),
        }
      }
      "img" => {
        if let Some(src) = clean_src(child.attr("src")) {
          sb.push_str(&image(child.attr("alt").unwrap_or(""), &src));
        }
      }
      "code" => sb.push_str(&inline_code(&child.text())),
      _ => {
        let (bold, italic, strike) = inline_marks(child, self.no_italic);
        if !bold && !italic && !strike {
          self.gather(child, sb);
        } else {
          let mut inner = String::new();
          self.gather(child, &mut inner);
          sb.push_str(&wrap_marks(&inner, bold, italic, strike));
        }
      }
    }
  }
}

/// `(?:language-|lang-)([\w+#-]+)` — the first match in a class string.
fn code_language(class: &str) -> String {
  let bytes = class.as_bytes();
  let mut i = 0;
  while i < bytes.len() {
    for prefix in ["language-", "lang-"] {
      if class[i..].starts_with(prefix) {
        let lang: String = class[i + prefix.len()..]
          .chars()
          .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '#' | '-'))
          .collect();
        if !lang.is_empty() {
          return lang;
        }
      }
    }
    i += class[i..].chars().next().map_or(1, char::len_utf8);
  }
  String::new()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn ordered_marker_shapes() {
    for m in ["1.", "(1)", "a)", "一、", "12.", "iv."] {
      assert!(is_ordered_marker(m), "{m}");
    }
    for m in ["·", "l", "§", "Ø", "", "1"] {
      assert!(!is_ordered_marker(m), "{m}");
    }
  }

  #[test]
  fn mso_level_reads_the_style() {
    assert_eq!(
      mso_level("margin:0;mso-list:l0 level2 lfo1").as_deref(),
      Some("2")
    );
    assert_eq!(mso_level("MSO-LIST:L0 LEVEL3 LFO1").as_deref(), Some("3"));
    assert_eq!(mso_level("mso-list:Ignore"), None);
    assert_eq!(mso_level("mso-list:l0 xlevel2"), None);
  }

  #[test]
  fn block_start_escapes() {
    assert_eq!(escape_block_start("1. x"), "1\\. x");
    assert_eq!(escape_block_start("# x"), "\\# x");
    assert_eq!(escape_block_start("#x"), "#x");
    assert_eq!(escape_block_start("- x"), "\\- x");
    assert_eq!(escape_block_start("---"), "\\---");
    assert_eq!(escape_block_start("> q"), "\\> q");
    assert_eq!(escape_block_start("1234567890. x"), "1234567890. x");
  }

  #[test]
  fn language_from_class() {
    assert_eq!(code_language(" language-rust"), "rust");
    assert_eq!(code_language("x lang-c++ y"), "c++");
    assert_eq!(code_language("highlight"), "");
  }
}
