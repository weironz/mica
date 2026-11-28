//! Code-block syntax highlighting: the one tokenizer behind the editor (desktop
//! FFI, Web wasm), the share page and the HTML export (gh#178).
//!
//! It used to live in Dart (`lib/editor/highlight.dart`), so the editor coloured
//! code and every exported or shared page showed it flat — two products from one
//! document. Now the grammars are data (`languages.json`) and the scanner is
//! here; Dart only maps a [`Role`] to a palette colour.
//!
//! The grammars' structural rules are JavaScript regex sources (that is where
//! they came from, and the parity fixture pins the behaviour they had there),
//! run by a small JS-semantics matcher (`regex.rs`).
//!
//! Offsets are UTF-16 code units, the unit of the editor's text model and of
//! every other offset the engine hands to Dart.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

mod regex;
use regex::Regex;

/// What a highlighted token IS; the caller picks the colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeToken {
  /// Start offset (UTF-16, inclusive).
  pub start: u32,
  /// End offset (UTF-16, exclusive).
  pub end: u32,
  /// What the run is.
  pub role: Role,
}

struct Grammar {
  keywords: HashSet<String>,
  line_comments: Vec<Vec<char>>,
  block_comments: bool,
  strings: Vec<char>,
  case_insensitive: bool,
  rules: Vec<Rule>,
}

struct Rule {
  pattern: Regex,
  role: Role,
  lead_only: bool,
}

struct Data {
  languages: Vec<String>,
  aliases: HashMap<String, String>,
  grammars: HashMap<String, Grammar>,
}

/// Walks the parsed `Value` by hand: serde-derived loaders for these shapes
/// cost the Web bundle ~20 KB for a one-time read of a file we ship.
fn data() -> &'static Data {
  static DATA: OnceLock<Data> = OnceLock::new();
  DATA.get_or_init(|| {
    let raw: Value =
      serde_json::from_str(include_str!("languages.json")).expect("languages.json is valid");
    let strs = |v: &Value| -> Vec<String> {
      v.as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
    };
    let flag = |v: &Value| v.as_bool().unwrap_or(false);
    let role = |name: &str| match name {
      "keyword" => Role::Keyword,
      "string" => Role::String,
      "comment" => Role::Comment,
      "number" => Role::Number,
      "function" => Role::Function,
      "key" => Role::Key,
      other => panic!("unknown highlight role {other:?}"),
    };
    let object = |v: &Value| v.as_object().cloned().unwrap_or_default();
    let grammars = object(&raw["grammars"])
      .into_iter()
      .map(|(name, g)| {
        let rules = g["rules"]
          .as_array()
          .into_iter()
          .flatten()
          .map(|r| {
            let pattern = r["pattern"].as_str().unwrap_or_default();
            Rule {
              pattern: Regex::new(pattern, false)
                .unwrap_or_else(|e| panic!("bad highlight rule {pattern:?}: {e}")),
              role: role(r["role"].as_str().unwrap_or_default()),
              lead_only: flag(&r["lead_only"]),
            }
          })
          .collect();
        let grammar = Grammar {
          keywords: strs(&g["keywords"]).into_iter().collect(),
          line_comments: strs(&g["line_comments"])
            .iter()
            .filter(|p| !p.is_empty())
            .map(|p| p.chars().collect())
            .collect(),
          block_comments: flag(&g["block_comments"]),
          strings: strs(&g["strings"])
            .iter()
            .filter_map(|s| s.chars().next())
            .collect(),
          case_insensitive: flag(&g["case_insensitive"]),
          rules,
        };
        (name, grammar)
      })
      .collect();
    let aliases = object(&raw["aliases"])
      .into_iter()
      .filter_map(|(k, v)| Some((k, v.as_str()?.to_string())))
      .collect();
    Data {
      languages: strs(&raw["languages"]),
      aliases,
      grammars,
    }
  })
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

fn starts_with(c: &[char], i: usize, pat: &[char]) -> bool {
  c.get(i..i + pat.len()) == Some(pat)
}

fn tokenize(code: &str, lang: &Grammar) -> Vec<CodeToken> {
  let c: Vec<char> = code.chars().collect();
  let n = c.len();
  // UTF-16 offset of every char index (and of the end).
  let mut u16_at = Vec::with_capacity(n + 1);
  let mut unit = 0u32;
  for ch in &c {
    u16_at.push(unit);
    unit += ch.len_utf16() as u32;
  }
  u16_at.push(unit);
  let mut out = Vec::new();
  let mut emit = |from: usize, to: usize, role: Role| {
    out.push(CodeToken {
      start: u16_at[from],
      end: u16_at[to],
      role,
    });
  };
  let mut i = 0usize;
  let mut line_lead = true;

  while i < n {
    let ch = c[i];
    let at_lead = line_lead;
    if ch == '\n' {
      line_lead = true;
    } else if ch != ' ' && ch != '\t' && ch != '-' {
      line_lead = false;
    }

    // Structural rules first: an HTML `<!-- -->` beats the string scanner on
    // the quotes inside it; a JSON key is claimed before it is just a string.
    let rule = lang.rules.iter().find_map(|rule| {
      if rule.lead_only && !at_lead {
        return None;
      }
      rule
        .pattern
        .match_at(&c, i)
        .filter(|&end| end > i)
        .map(|end| (end, rule.role))
    });
    if let Some((end, role)) = rule {
      emit(i, end, role);
      i = end;
      line_lead = false;
      continue;
    }

    if lang.block_comments && starts_with(&c, i, &['/', '*']) {
      let stop = (i + 2..n.saturating_sub(1))
        .find(|&j| c[j] == '*' && c[j + 1] == '/')
        .map_or(n, |j| j + 2);
      emit(i, stop, Role::Comment);
      i = stop;
      continue;
    }

    if lang.line_comments.iter().any(|p| starts_with(&c, i, p)) {
      let stop = (i..n).find(|&j| c[j] == '\n').unwrap_or(n);
      emit(i, stop, Role::Comment);
      i = stop;
      continue;
    }

    if lang.strings.contains(&ch) {
      let mut j = i + 1;
      while j < n {
        if c[j] == '\\' {
          j += 2;
          continue;
        }
        if c[j] == ch {
          j += 1;
          break;
        }
        if c[j] == '\n' {
          break; // unterminated: stop at the line end
        }
        j += 1;
      }
      let j = j.min(n);
      emit(i, j, Role::String);
      i = j;
      continue;
    }

    if ch.is_ascii_digit() {
      let mut j = i + 1;
      while j < n && (c[j].is_ascii_hexdigit() || matches!(c[j], 'x' | 'X' | '.' | '_')) {
        j += 1;
      }
      emit(i, j, Role::Number);
      i = j;
      continue;
    }

    if ch.is_ascii_alphabetic() || ch == '_' {
      let mut j = i + 1;
      while j < n && (c[j].is_ascii_alphanumeric() || c[j] == '_') {
        j += 1;
      }
      let word: String = c[i..j].iter().collect();
      let keyword = if lang.case_insensitive {
        lang.keywords.contains(&word.to_ascii_lowercase())
      } else {
        lang.keywords.contains(&word)
      };
      if keyword {
        emit(i, j, Role::Keyword);
      } else if j < n && c[j] == '(' {
        emit(i, j, Role::Function);
      }
      i = j;
      continue;
    }

    i += 1; // plain
  }
  out
}

// ---------------------------------------------------------------------------
// Detection
// ---------------------------------------------------------------------------

/// A detection pattern (JS source, multiline?) compiled once.
fn re(src: &'static str, multiline: bool) -> &'static Regex {
  static CACHE: OnceLock<Mutex<HashMap<(&'static str, bool), &'static Regex>>> = OnceLock::new();
  let mut cache = CACHE
    .get_or_init(Default::default)
    .lock()
    .unwrap_or_else(|e| e.into_inner());
  cache.entry((src, multiline)).or_insert_with(|| {
    let regex =
      Regex::new(src, multiline).unwrap_or_else(|e| panic!("bad detection pattern {src:?}: {e}"));
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
  signature(&code.chars().collect::<Vec<_>>())
}

fn signature(code: &[char]) -> Option<&'static str> {
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
  let chars: Vec<char> = c.chars().collect();
  let has = |p: &'static str| re(p, false).is_match(&chars);
  let has_line = |p: &'static str| re(p, true).is_match(&chars);

  if let Some(strong) = signature(&chars) {
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
  if has_line(r"^---\s*$") || re(r"^[ \t]*-?[ \t]*[\w.-]+:(?:[ \t]|$)", true).count(&chars) >= 2 {
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
  if has(r"\$\((?!\()") || has(r"`[^`\n$]+`") {
    weak += 1;
  }
  if has(r"\s(?:&&|\|\|)\s") {
    weak += 1;
  }
  if has_line(r"^\s*(?:export|source|local|readonly)\s+\w") {
    weak += 1;
  }
  // VAR=value with no spaces (Python/Ruby space it); `==` is a comparison.
  if has_line(r"^\s*[A-Za-z_]\w*=(?!=)\S") {
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
