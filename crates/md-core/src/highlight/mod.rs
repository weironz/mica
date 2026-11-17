//! Code-block syntax highlighting: the one tokenizer behind the editor (desktop
//! FFI, Web wasm), the share page and the HTML export (gh#178).
//!
//! It used to live in Dart (`lib/editor/highlight.dart`), so the editor coloured
//! code and every exported or shared page showed it flat — two products from one
//! document. Now the grammars are data ([`languages.json`]) and the scanner is
//! here; Dart only maps a [`Role`] to a palette colour.
//!
//! The grammars' structural rules are JavaScript-flavoured regex sources (that is
//! where they came from, and the parity fixture pins the behaviour they had
//! there). [`compile`] translates the few JS-isms regex-lite lacks: `\s` is JS's
//! Unicode whitespace set, `.` excludes JS line terminators, and a trailing
//! `(?=…)` / `(?!…)` is checked separately at the match end — sound because in
//! every rule the body cannot end earlier by backtracking (it is followed by a
//! character class the tail never starts with).
//!
//! Offsets are UTF-16 code units, the unit of the editor's text model and of
//! every other offset the engine hands to Dart.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use regex_lite::Regex;
use serde::{Deserialize, Serialize};

/// What a highlighted token IS; the caller picks the colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
  /// A language keyword.
  Keyword,
  /// A string literal.
  String,
  /// A comment.
  Comment,
  /// A numeric literal.
  Number,
  /// A call, tag, selector or similar "named thing".
  Function,
  /// A mapping key / property name.
  Key,
}

impl Role {
  /// Stable short name (`keyword`, `string`, …), also the HTML class suffix.
  pub fn as_str(self) -> &'static str {
    match self {
      Role::Keyword => "keyword",
      Role::String => "string",
      Role::Comment => "comment",
      Role::Number => "number",
      Role::Function => "function",
      Role::Key => "key",
    }
  }
}

/// One coloured run, in UTF-16 code units; text between tokens is plain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeToken {
  /// Start offset (UTF-16, inclusive).
  pub start: u32,
  /// End offset (UTF-16, exclusive).
  pub end: u32,
  /// What the run is.
  pub role: Role,
}

#[derive(Deserialize)]
struct RawData {
  languages: Vec<String>,
  aliases: HashMap<String, String>,
  grammars: HashMap<String, RawGrammar>,
}

#[derive(Deserialize)]
struct RawGrammar {
  keywords: Vec<String>,
  line_comments: Vec<String>,
  block_comments: bool,
  strings: Vec<String>,
  case_insensitive: bool,
  rules: Vec<RawRule>,
}

#[derive(Deserialize)]
struct RawRule {
  pattern: String,
  role: Role,
  lead_only: bool,
}

struct Grammar {
  keywords: HashSet<String>,
  line_comments: Vec<String>,
  block_comments: bool,
  strings: Vec<u8>,
  case_insensitive: bool,
  rules: Vec<Rule>,
}

struct Rule {
  pattern: Pattern,
  role: Role,
  lead_only: bool,
}

struct Data {
  languages: Vec<String>,
  aliases: HashMap<String, String>,
  grammars: HashMap<String, Grammar>,
}

fn data() -> &'static Data {
  static DATA: OnceLock<Data> = OnceLock::new();
  DATA.get_or_init(|| {
    let raw: RawData =
      serde_json::from_str(include_str!("languages.json")).expect("languages.json is valid");
    let grammars = raw
      .grammars
      .into_iter()
      .map(|(name, g)| {
        let grammar = Grammar {
          keywords: g.keywords.into_iter().collect(),
          line_comments: g.line_comments,
          block_comments: g.block_comments,
          strings: g.strings.iter().filter_map(|s| s.bytes().next()).collect(),
          case_insensitive: g.case_insensitive,
          rules: g
            .rules
            .into_iter()
            .map(|r| Rule {
              pattern: Pattern::new(&r.pattern),
              role: r.role,
              lead_only: r.lead_only,
            })
            .collect(),
        };
        (name, grammar)
      })
      .collect();
    Data {
      languages: raw.languages,
      aliases: raw.aliases,
      grammars,
    }
  })
}

// ---------------------------------------------------------------------------
// JS regex sources → regex-lite
// ---------------------------------------------------------------------------

/// JavaScript's `\s`, as class contents.
const JS_WS: &str =
  r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// Translate a JS (non-unicode-mode) regex source into regex-lite syntax.
fn translate(src: &str) -> String {
  let mut out = String::with_capacity(src.len() + 16);
  let mut chars = src.chars().peekable();
  let mut in_class = false;
  while let Some(c) = chars.next() {
    match c {
      '\\' => {
        let Some(e) = chars.next() else {
          out.push_str(r"\\");
          break;
        };
        match (e, in_class) {
          ('s', true) => out.push_str(JS_WS),
          ('s', false) => {
            out.push('[');
            out.push_str(JS_WS);
            out.push(']');
          }
          ('S', false) => {
            out.push_str("[^");
            out.push_str(JS_WS);
            out.push(']');
          }
          ('S', true) => panic!("\\S inside a class is not supported: {src}"),
          _ => {
            out.push('\\');
            out.push(e);
          }
        }
      }
      '[' if !in_class => {
        // `[\s\S]` is JS's "any character".
        let rest: String = chars.clone().take(5).collect();
        if rest == r"\s\S]" {
          for _ in 0..5 {
            chars.next();
          }
          out.push_str("(?s:.)");
          continue;
        }
        in_class = true;
        out.push('[');
        if chars.peek() == Some(&'^') {
          out.push('^');
          chars.next();
        }
        // A leading `]` is a literal in neither dialect; JS treats `[]` as empty.
      }
      ']' if in_class => {
        in_class = false;
        out.push(']');
      }
      '.' if !in_class => out.push_str(r"[^\n\r\x{2028}\x{2029}]"),
      _ => out.push(c),
    }
  }
  out
}

/// Index of the `(` that opens a top-level group ending exactly at the end of
/// `src`, when that group is a lookahead: returns (open index, positive).
fn trailing_lookahead(src: &str) -> Option<(usize, bool)> {
  let b = src.as_bytes();
  if !src.ends_with(')') {
    return None;
  }
  // Walk forward tracking depth; remember where each top-level group opens.
  let mut depth = 0usize;
  let mut in_class = false;
  let mut i = 0;
  let mut last_open = None;
  while i < b.len() {
    match b[i] {
      b'\\' => {
        i += 2;
        continue;
      }
      b'[' if !in_class => in_class = true,
      b']' if in_class => in_class = false,
      b'(' if !in_class => {
        if depth == 0 {
          last_open = Some(i);
        }
        depth += 1;
      }
      b')' if !in_class => {
        depth -= 1;
        if depth == 0 && i == b.len() - 1 {
          let open = last_open?;
          let head = &src[open..];
          if head.starts_with("(?=") {
            return Some((open, true));
          }
          if head.starts_with("(?!") {
            return Some((open, false));
          }
          return None;
        }
      }
      _ => {}
    }
    i += 1;
  }
  None
}

/// A JS regex source, matched anchored at a position (`matchAsPrefix`).
struct Pattern {
  body: Regex,
  /// `^`-anchored source: only ever matches at offset 0.
  at_start: bool,
  /// A trailing lookahead, checked at the body's end: (assertion, positive).
  tail: Option<(Box<Pattern>, bool)>,
}

impl Pattern {
  fn new(src: &str) -> Self {
    let (body_src, tail) = match trailing_lookahead(src) {
      Some((open, positive)) => {
        let inner = &src[open + 3..src.len() - 1];
        (
          &src[..open],
          Some((Box::new(Pattern::new(inner)), positive)),
        )
      }
      None => (src, None),
    };
    let body = Regex::new(&format!("^(?:{})", translate(body_src)))
      .unwrap_or_else(|e| panic!("bad highlight pattern {src:?}: {e}"));
    Pattern {
      body,
      at_start: body_src.starts_with('^'),
      tail,
    }
  }

  /// Byte end of a match starting exactly at byte `i`, if any.
  fn match_at(&self, code: &str, i: usize) -> Option<usize> {
    if self.at_start && i != 0 {
      return None;
    }
    let m = self.body.find(&code[i..])?;
    let end = i + m.end();
    if let Some((tail, positive)) = &self.tail {
      if tail.match_at(code, end).is_some() != *positive {
        return None;
      }
    }
    Some(end)
  }
}

// ---------------------------------------------------------------------------
// Languages
// ---------------------------------------------------------------------------

/// The language choices offered for a code block, `auto` and `plaintext` first.
pub fn code_languages() -> &'static [String] {
  &data().languages
}

/// The canonical name for a fence label / picker choice: `py` → `python`.
/// Unknown names pass through (trimmed, lowercased) and simply won't highlight.
pub fn canonical_code_language(name: &str) -> String {
  let n = dart_trim(name).to_lowercase();
  match data().aliases.get(&n) {
    Some(canonical) => canonical.clone(),
    None => n,
  }
}

/// The effective language: the explicit choice, or the detected one when the
/// choice is empty or `auto`.
pub fn resolve_code_language(code: &str, selected: Option<&str>) -> String {
  let choice = canonical_code_language(selected.unwrap_or(""));
  if !choice.is_empty() && choice != "auto" {
    return choice;
  }
  detect_code_language(code)
}

fn is_dart_ws(c: char) -> bool {
  matches!(c,
    '\u{9}'..='\u{D}' | ' ' | '\u{85}' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}'
    | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
}

fn dart_trim(s: &str) -> &str {
  s.trim_matches(is_dart_ws)
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

/// Highlight `code` as `language` (any spelling [`canonical_code_language`]
/// accepts). Unknown languages and `plaintext` yield no tokens.
pub fn highlight_code(code: &str, language: &str) -> Vec<CodeToken> {
  let name = canonical_code_language(language);
  if name == "plaintext" {
    return Vec::new();
  }
  let Some(lang) = data().grammars.get(&name) else {
    return Vec::new();
  };
  tokenize(code, lang)
}

fn utf16_len(s: &str) -> u32 {
  s.chars().map(|c| c.len_utf16() as u32).sum()
}

fn tokenize(code: &str, lang: &Grammar) -> Vec<CodeToken> {
  let b = code.as_bytes();
  let n = b.len();
  let mut out = Vec::new();
  let mut i = 0usize; // byte offset
  let mut u = 0u32; // UTF-16 offset of `i`
  let mut line_lead = true;
  let emit = |out: &mut Vec<CodeToken>, from: usize, to: usize, u: &mut u32, role: Role| {
    let len = utf16_len(&code[from..to]);
    out.push(CodeToken {
      start: *u,
      end: *u + len,
      role,
    });
    *u += len;
  };

  while i < n {
    let ch = b[i];
    let at_lead = line_lead;
    if ch == b'\n' {
      line_lead = true;
    } else if ch != b' ' && ch != b'\t' && ch != b'-' {
      line_lead = false;
    }

    // Structural rules first: an HTML `<!-- -->` beats the string scanner on
    // the quotes inside it; a JSON key is claimed before it is just a string.
    let mut matched = false;
    for rule in &lang.rules {
      if rule.lead_only && !at_lead {
        continue;
      }
      let Some(end) = rule.pattern.match_at(code, i) else {
        continue;
      };
      if end == i {
        continue;
      }
      emit(&mut out, i, end, &mut u, rule.role);
      i = end;
      line_lead = false;
      matched = true;
      break;
    }
    if matched {
      continue;
    }

    let rest = &code[i..];
    if lang.block_comments && rest.starts_with("/*") {
      let stop = code[i + 2..].find("*/").map_or(n, |e| i + 2 + e + 2);
      emit(&mut out, i, stop, &mut u, Role::Comment);
      i = stop;
      continue;
    }

    if lang
      .line_comments
      .iter()
      .any(|p| !p.is_empty() && rest.starts_with(p.as_str()))
    {
      let stop = code[i..].find('\n').map_or(n, |e| i + e);
      emit(&mut out, i, stop, &mut u, Role::Comment);
      i = stop;
      continue;
    }

    if lang.strings.contains(&ch) {
      let mut j = i + 1;
      while j < n {
        let c = b[j];
        if c == b'\\' {
          // Skip the escaped character, whatever its width.
          j += 1;
          if j < n {
            j += code[j..].chars().next().map_or(1, char::len_utf8);
          }
          continue;
        }
        if c == ch {
          j += 1;
          break;
        }
        if c == b'\n' {
          break; // unterminated: stop at the line end
        }
        j += 1;
      }
      let j = j.min(n);
      emit(&mut out, i, j, &mut u, Role::String);
      i = j;
      continue;
    }

    if ch.is_ascii_digit() {
      let mut j = i + 1;
      while j < n && (b[j].is_ascii_hexdigit() || matches!(b[j], b'x' | b'X' | b'.' | b'_')) {
        j += 1;
      }
      emit(&mut out, i, j, &mut u, Role::Number);
      i = j;
      continue;
    }

    if ch.is_ascii_alphabetic() || ch == b'_' {
      let mut j = i + 1;
      while j < n && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
        j += 1;
      }
      let word = &code[i..j];
      let keyword = if lang.case_insensitive {
        lang.keywords.contains(&word.to_ascii_lowercase())
      } else {
        lang.keywords.contains(word)
      };
      if keyword {
        emit(&mut out, i, j, &mut u, Role::Keyword);
      } else if j < n && b[j] == b'(' {
        emit(&mut out, i, j, &mut u, Role::Function);
      } else {
        u += (j - i) as u32;
      }
      i = j;
      continue;
    }

    // Plain: one whole character (a non-ASCII one never starts a token).
    let c = code[i..].chars().next().expect("in bounds");
    i += c.len_utf8();
    u += c.len_utf16() as u32;
  }
  out
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// A JS regex (source, multiline) compiled once.
fn re(src: &'static str, multiline: bool) -> &'static Regex {
  static CACHE: OnceLock<Mutex<HashMap<(&'static str, bool), &'static Regex>>> = OnceLock::new();
  let mut cache = CACHE
    .get_or_init(Default::default)
    .lock()
    .unwrap_or_else(|e| e.into_inner());
  cache.entry((src, multiline)).or_insert_with(|| {
    let flags = if multiline { "(?m)" } else { "" };
    let regex = Regex::new(&format!("{flags}{}", translate(src)))
      .unwrap_or_else(|e| panic!("bad detection pattern {src:?}: {e}"));
    Box::leak(Box::new(regex))
  })
}

/// The language whose *structural syntax* the code plainly uses, or `None`
/// when nothing is conclusive.
///
/// Much stricter than [`detect_code_language`]: every signature is syntax that
/// would be an error in most other languages. Used to overrule the language a
/// *pasted* fence claims about itself, so a false positive silently mislabels
/// someone's code; `None` is always the safe answer.
pub fn strong_language_signature(code: &str) -> Option<&'static str> {
  let has = |p: &'static str| re(p, true).is_match(code);
  // A shebang says what the file IS.
  if has(r"^#!.*\b(bash|sh|zsh|ksh)\b") {
    return Some("bash");
  }
  if has(r"^#!.*\bpython[\d.]*\b") {
    return Some("python");
  }
  if has(r"^#!.*\bnode\b") {
    return Some("javascript");
  }
  // `def f(...):` / `class C(...):` / `elif`: a colon-terminated block header.
  if has(r"^\s*(?:async\s+)?def\s+\w+\s*\([^)]*\)\s*(?:->[^:]+)?:")
    || has(r"^\s*elif\s+.+:\s*$")
    || has(r"^\s*class\s+\w+\s*(?:\([\w., ]*\))?\s*:\s*$")
  {
    return Some("python");
  }
  if has(r"\bfn\s+\w+\s*(?:<[^>]*>)?\s*\([^)]*\)")
    && (has(r"\blet\s+mut\b") || has(r"\w+!\s*\(") || has(r"->\s*\w"))
  {
    return Some("rust");
  }
  if has(r"^\s*package\s+\w+\s*$") && has(r"^\s*func\s+\w+\s*\(") {
    return Some("go");
  }
  // Deliberately absent: `#include` (C and C++ share it) and `public class`
  // (C#, Java, Kotlin): a signature that can't name ONE language isn't one.
  None
}

/// Heuristic language detection from content signatures; `plaintext` when
/// nothing fits.
pub fn detect_code_language(code: &str) -> String {
  detect(code).to_string()
}

fn detect(c: &str) -> &'static str {
  if c.is_empty() {
    return "plaintext";
  }
  let has = |p: &'static str| re(p, false).is_match(c);
  let has_line = |p: &'static str| re(p, true).is_match(c);

  if let Some(strong) = strong_language_signature(c) {
    return strong;
  }

  // Shell, tier A: a session PROMPT or a HERE-DOC is shell-exclusive, so it is
  // checked before the weaker keyword heuristics. Command NAMES are not
  // consulted: keying on them is how highlighters over-claim bash on prose.
  if has_line(r"[\w.-]+@[\w.-]+:[^\n]*[$#%]")
    || has_line(r"^\s*\[[^\]]+\]:?\s*[$#%]\s")
    || has_line(r#"<<-?\s*[\x27"]?\w+\s*$"#)
  {
    return "bash";
  }

  if has(r"^\s*<\?xml") || has(r"<\w+[^>]*>.*</\w+>") || has(r"<!DOCTYPE") {
    return "html";
  }
  // CSS before YAML: a rule body's `prop: value;` lines look like YAML keys.
  if has(r"\{[^{}]*[\w-]+\s*:\s*[^;{}]+;")
    || has_line(r"^\s*@(?:media|import|keyframes|font-face)\b")
  {
    return "css";
  }
  // YAML: `key:` at the head of a line IS the language. Two of them, or a
  // document marker. Above Python's weak rules: a compose file's
  // `command: python -c "print(1)"` must not read as Python.
  if has_line(r"^---\s*$")
    || re(r"^[ \t]*-?[ \t]*[\w.-]+:(?:[ \t]|$)", true)
      .find_iter(c)
      .count()
      >= 2
  {
    return "yaml";
  }
  if has(r"\bfn\s+\w+") && (c.contains("->") || c.contains("let mut") || c.contains("println!")) {
    return "rust";
  }
  if has(r"\bfunc\s+\w+") || has(r"\bpackage\s+main") || c.contains(":=") {
    return "go";
  }
  if has(r"\bdef\s+\w+\s*\(") || has(r"^\s*import\s+\w+\s*$") || c.contains("print(") {
    return "python";
  }
  if c.contains("#include") || has(r"\bstd::") {
    return if c.contains("std::") || c.contains("template") {
      "cpp"
    } else {
      "c"
    };
  }
  if has(r"\b(public|private)\s+(static\s+)?(class|void|int)\b") || c.contains("System.out") {
    return "java";
  }
  if has(r"\b(SELECT|INSERT|UPDATE|DELETE|CREATE)\b") {
    return "sql";
  }
  if has(r"\b(const|let|var|function)\b") && (c.contains("=>") || c.contains("){")) {
    let typed = c.contains(": ")
      && (c.contains("interface ") || c.contains(": string") || c.contains(": number"));
    return if typed { "typescript" } else { "javascript" };
  }
  if has(r"\bvoid\s+main\b") || c.contains("Widget build(") {
    return "dart";
  }
  let lead = c.trim_start_matches(is_dart_ws);
  if lead.starts_with('{') || lead.starts_with('[') {
    return "json";
  }

  // Shell, tiers B/C: weaker syntax, checked LAST so real Python/Ruby/Perl/JS
  // (`#` comments, `$` sigils, template literals, backticks) win first. A
  // strong signal stands alone; weak ones need two together.
  let strong_shell = has(r"\|\s*(?:grep|awk|sed|xargs|head|tail|sort|uniq|wc|cut|tr|tee|less)\b")
    || has(r"2>&1")
    || has(r">\s*/dev/null")
    || has(r"&>")
    || has(r"\$\(\(")
    || has(r"<<<")
    || has(r"\$\{[#!]?\w+(?:\[[^\]]*\]|:[-=+?]|##?|%%?|/)")
    || (has_line(r"^\s*(?:if|elif|for|while|until|case)\b") && has(r"\b(?:fi|done|esac)\b"));
  if strong_shell {
    return "bash";
  }

  let mut weak = 0;
  // `$(cmd)` (not `$((`) or a backtick run with no `$` inside (not a template).
  if has(r"\$\((?:[^(]|$)") || has(r"`[^`\n$]+`") {
    weak += 1;
  }
  if has(r"\s(?:&&|\|\|)\s") {
    weak += 1;
  }
  if has_line(r"^\s*(?:export|source|local|readonly)\s+\w") {
    weak += 1;
  }
  // VAR=value with no spaces (Python/Ruby space it); `==` is a comparison.
  if has_line(r"^\s*[A-Za-z_]\w*=[^=\s]") {
    weak += 1;
  }
  if has(r"\becho\s+") {
    weak += 1;
  }
  // A gated command list: ONE signal, only next to a shell operator.
  if (has(r"[|]") || has(r"\s&&\s") || has(r">") || has_line(r"\\\s*$"))
    && has_line(
      r"^\s*(?:sudo|apt|apt-get|yum|dnf|systemctl|service|journalctl|docker|kubectl|helm|git|curl|wget|ssh|scp|rsync|chmod|chown|nvidia-smi|dcgmi|pip3?|python3?)\b",
    )
  {
    weak += 1;
  }
  if weak >= 2 {
    return "bash";
  }
  "plaintext"
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn every_grammar_compiles() {
    assert!(data().grammars.len() > 40);
  }

  #[test]
  fn trailing_lookahead_is_split() {
    let p = Pattern::new(r"[A-Za-z_][A-Za-z0-9_]*(?=\s*:)");
    assert_eq!(p.match_at("key: 1", 0), Some(3));
    assert_eq!(p.match_at("key = 1", 0), None);
    let p = Pattern::new(r"\.?[A-Za-z0-9_%./\-]+(?=\s*:(?!=))");
    assert_eq!(p.match_at("all: x", 0), Some(3));
    assert_eq!(p.match_at("x := 1", 0), None);
  }

  #[test]
  fn offsets_are_utf16() {
    let t = highlight_code("// é😀\nlet", "javascript");
    assert_eq!(
      t[0],
      CodeToken {
        start: 0,
        end: 6,
        role: Role::Comment
      }
    );
    assert_eq!(
      t[1],
      CodeToken {
        start: 7,
        end: 10,
        role: Role::Keyword
      }
    );
  }
}
