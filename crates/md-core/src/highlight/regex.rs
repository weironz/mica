//! A small backtracking matcher for the JavaScript regex subset the grammars
//! and detection heuristics are written in.
//!
//! Hand-written rather than a crate: the rules came from a JS engine and the
//! parity fixture pins JS behaviour, which needs lookahead (`(?=…)`, `(?!…)`)
//! and JS's `\s` / `.` / `^$` line semantics; and the general-purpose engines
//! cost the Web bundle ~50 KB gzip for a few dozen patterns.
//!
//! Supported: literals and escapes (`\d \w \s \b` and negations, `\n \t \r
//! \f \v \xHH \uHHHH`, escaped metacharacters), classes with ranges and
//! negation, `.`, groups (capturing ones are matched as non-capturing),
//! alternation, `* + ? {n} {n,} {n,m}` and their lazy forms, `^ $` (with the
//! multiline flag), lookahead. Matching walks `char`s; priority order is JS's
//! (leftmost alternative, greedy-first), so the first match found is the one a
//! JS engine reports. The backtrack stack lives on the heap: a long line under
//! `[^\n]*` costs memory, never call depth.

/// A compiled pattern.
pub(crate) struct Regex {
  prog: Vec<Inst>,
  multiline: bool,
  /// Starts with `^` outside multiline mode: only position 0 can match.
  at_start: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Builtin {
  Digit,
  Word,
  Space,
}

impl Builtin {
  fn matches(self, c: char) -> bool {
    match self {
      Builtin::Digit => c.is_ascii_digit(),
      Builtin::Word => c.is_ascii_alphanumeric() || c == '_',
      Builtin::Space => is_js_space(c),
    }
  }
}

fn is_js_space(c: char) -> bool {
  matches!(
    c,
    '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
      ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
  )
}

fn is_line_terminator(c: char) -> bool {
  matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn is_word(c: char) -> bool {
  c.is_ascii_alphanumeric() || c == '_'
}

/// A character set: ranges plus builtins, each possibly negated.
#[derive(Clone)]
struct Class {
  negate: bool,
  ranges: Vec<(char, char)>,
  builtins: Vec<(Builtin, bool)>, // (set, negated)
}

impl Class {
  fn matches(&self, c: char) -> bool {
    let hit = self.ranges.iter().any(|&(a, b)| a <= c && c <= b)
      || self
        .builtins
        .iter()
        .any(|&(set, neg)| set.matches(c) != neg);
    hit != self.negate
  }
}

enum Inst {
  Char(char),
  Class(Class),
  /// `.`: anything but a JS line terminator.
  Any,
  Bol,
  Eol,
  WordBoundary(bool),
  /// Try `.0` first, `.1` on backtrack.
  Split(usize, usize),
  Jmp(usize),
  /// A lookahead: sub-program, positive?
  Look(Box<Regex>, bool),
  Match,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

enum Node {
  Empty,
  Char(char),
  Class(Class),
  Any,
  Bol,
  Eol,
  WordBoundary(bool),
  Look(Box<Node>, bool),
  Concat(Vec<Node>),
  Alt(Vec<Node>),
  Repeat {
    node: Box<Node>,
    min: u32,
    max: Option<u32>,
    greedy: bool,
  },
}

struct Parser<'a> {
  src: &'a [char],
  pos: usize,
}

type ParseResult<T> = Result<T, String>;

impl Parser<'_> {
  fn peek(&self) -> Option<char> {
    self.src.get(self.pos).copied()
  }

  fn eat(&mut self, c: char) -> bool {
    if self.peek() == Some(c) {
      self.pos += 1;
      true
    } else {
      false
    }
  }

  fn alt(&mut self) -> ParseResult<Node> {
    let mut branches = vec![self.concat()?];
    while self.eat('|') {
      branches.push(self.concat()?);
    }
    Ok(if branches.len() == 1 {
      branches.pop().unwrap()
    } else {
      Node::Alt(branches)
    })
  }

  fn concat(&mut self) -> ParseResult<Node> {
    let mut items = Vec::new();
    while let Some(c) = self.peek() {
      if c == '|' || c == ')' {
        break;
      }
      let atom = self.atom()?;
      items.push(self.quantified(atom)?);
    }
    Ok(match items.len() {
      0 => Node::Empty,
      1 => items.pop().unwrap(),
      _ => Node::Concat(items),
    })
  }

  fn quantified(&mut self, atom: Node) -> ParseResult<Node> {
    let simple = match self.peek() {
      Some('*') => Some((0, None)),
      Some('+') => Some((1, None)),
      Some('?') => Some((0, Some(1))),
      _ => None,
    };
    let (min, max) = match simple {
      Some(bounds) => {
        self.pos += 1;
        bounds
      }
      None if self.peek() == Some('{') => match self.braces() {
        Some(bounds) => bounds,
        None => return Ok(atom), // a literal `{` (JS annex B)
      },
      None => return Ok(atom),
    };
    let greedy = !self.eat('?');
    Ok(Node::Repeat {
      node: Box::new(atom),
      min,
      max,
      greedy,
    })
  }

  /// `{n}`, `{n,}`, `{n,m}` (consumed), or None if this `{` is literal.
  fn braces(&mut self) -> Option<(u32, Option<u32>)> {
    let start = self.pos;
    self.pos += 1;
    let num = |p: &mut Self| -> Option<u32> {
      let from = p.pos;
      while p.peek().is_some_and(|c| c.is_ascii_digit()) {
        p.pos += 1;
      }
      p.src[from..p.pos].iter().collect::<String>().parse().ok()
    };
    let parsed = (|| {
      let min = num(self)?;
      let max = if self.eat(',') {
        if self.peek() == Some('}') {
          None
        } else {
          Some(num(self)?)
        }
      } else {
        Some(min)
      };
      self.eat('}').then_some((min, max))
    })();
    if parsed.is_none() {
      self.pos = start;
    }
    parsed
  }

  fn atom(&mut self) -> ParseResult<Node> {
    let c = self.peek().ok_or("unexpected end")?;
    self.pos += 1;
    Ok(match c {
      '.' => Node::Any,
      '^' => Node::Bol,
      '$' => Node::Eol,
      '[' => Node::Class(self.class()?),
      '(' => {
        let node = if self.eat('?') {
          match self.peek() {
            Some(':') => {
              self.pos += 1;
              self.alt()?
            }
            Some(k @ ('=' | '!')) => {
              self.pos += 1;
              Node::Look(Box::new(self.alt()?), k == '=')
            }
            _ => return Err("unsupported group".into()),
          }
        } else {
          self.alt()?
        };
        if !self.eat(')') {
          return Err("unclosed group".into());
        }
        node
      }
      '\\' => self.escape()?,
      '*' | '+' | '?' => return Err(format!("nothing to repeat at {}", self.pos)),
      other => Node::Char(other),
    })
  }

  fn hex(&mut self, digits: usize) -> ParseResult<char> {
    let s: String = self
      .src
      .get(self.pos..self.pos + digits)
      .ok_or("short escape")?
      .iter()
      .collect();
    self.pos += digits;
    u32::from_str_radix(&s, 16)
      .ok()
      .and_then(char::from_u32)
      .ok_or_else(|| "bad hex escape".into())
  }

  /// An escape outside a class (the `\` already consumed).
  fn escape(&mut self) -> ParseResult<Node> {
    let c = self.peek().ok_or("trailing backslash")?;
    self.pos += 1;
    let builtin = |set, neg| {
      Node::Class(Class {
        negate: false,
        ranges: vec![],
        builtins: vec![(set, neg)],
      })
    };
    Ok(match c {
      'd' => builtin(Builtin::Digit, false),
      'D' => builtin(Builtin::Digit, true),
      'w' => builtin(Builtin::Word, false),
      'W' => builtin(Builtin::Word, true),
      's' => builtin(Builtin::Space, false),
      'S' => builtin(Builtin::Space, true),
      'b' => Node::WordBoundary(true),
      'B' => Node::WordBoundary(false),
      _ => Node::Char(self.escaped_char(c)?),
    })
  }

  /// The literal an escape letter stands for (`\n`, `\x27`, `\.`).
  fn escaped_char(&mut self, c: char) -> ParseResult<char> {
    Ok(match c {
      'n' => '\n',
      't' => '\t',
      'r' => '\r',
      'f' => '\u{C}',
      'v' => '\u{B}',
      '0' => '\0',
      'x' => self.hex(2)?,
      'u' => self.hex(4)?,
      other => other,
    })
  }

  fn class(&mut self) -> ParseResult<Class> {
    let mut class = Class {
      negate: self.eat('^'),
      ranges: vec![],
      builtins: vec![],
    };
    loop {
      let c = self.peek().ok_or("unclosed class")?;
      self.pos += 1;
      if c == ']' {
        return Ok(class);
      }
      let lo = if c == '\\' {
        let e = self.peek().ok_or("unclosed class")?;
        self.pos += 1;
        let set = match e {
          'd' | 'D' => Some(Builtin::Digit),
          'w' | 'W' => Some(Builtin::Word),
          's' | 'S' => Some(Builtin::Space),
          _ => None,
        };
        if let Some(set) = set {
          class.builtins.push((set, e.is_ascii_uppercase()));
          continue;
        }
        if e == 'b' {
          '\u{8}'
        } else {
          self.escaped_char(e)?
        }
      } else {
        c
      };
      // A range `a-z`; a `-` before `]` is literal.
      if self.peek() == Some('-') && self.src.get(self.pos + 1).is_some_and(|&n| n != ']') {
        self.pos += 1;
        let h = self.peek().ok_or("unclosed class")?;
        self.pos += 1;
        let hi = if h == '\\' {
          let e = self.peek().ok_or("unclosed class")?;
          self.pos += 1;
          self.escaped_char(e)?
        } else {
          h
        };
        class.ranges.push((lo, hi));
      } else {
        class.ranges.push((lo, lo));
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Compilation
// ---------------------------------------------------------------------------

fn emit(node: &Node, prog: &mut Vec<Inst>, multiline: bool) -> Result<(), String> {
  match node {
    Node::Empty => {}
    Node::Char(c) => prog.push(Inst::Char(*c)),
    Node::Class(c) => prog.push(Inst::Class(c.clone())),
    Node::Any => prog.push(Inst::Any),
    Node::Bol => prog.push(Inst::Bol),
    Node::Eol => prog.push(Inst::Eol),
    Node::WordBoundary(b) => prog.push(Inst::WordBoundary(*b)),
    Node::Look(inner, positive) => {
      let mut sub = Vec::new();
      emit(inner, &mut sub, multiline)?;
      sub.push(Inst::Match);
      let sub = Regex {
        prog: sub,
        multiline,
        at_start: false,
      };
      prog.push(Inst::Look(Box::new(sub), *positive));
    }
    Node::Concat(items) => {
      for item in items {
        emit(item, prog, multiline)?;
      }
    }
    Node::Alt(branches) => {
      let mut jumps = Vec::new();
      for (i, branch) in branches.iter().enumerate() {
        if i + 1 < branches.len() {
          let split = prog.len();
          prog.push(Inst::Split(split + 1, 0));
          emit(branch, prog, multiline)?;
          jumps.push(prog.len());
          prog.push(Inst::Jmp(0));
          let next = prog.len();
          prog[split] = Inst::Split(split + 1, next);
        } else {
          emit(branch, prog, multiline)?;
        }
      }
      let end = prog.len();
      for j in jumps {
        prog[j] = Inst::Jmp(end);
      }
    }
    Node::Repeat {
      node,
      min,
      max,
      greedy,
    } => {
      for _ in 0..*min {
        emit(node, prog, multiline)?;
      }
      let split = |prog: &mut Vec<Inst>, at: usize, body: usize, out: usize| {
        prog[at] = if *greedy {
          Inst::Split(body, out)
        } else {
          Inst::Split(out, body)
        };
      };
      match max {
        None => {
          // L: split(body, out); body; jmp L
          let at = prog.len();
          prog.push(Inst::Jmp(0));
          emit(node, prog, multiline)?;
          prog.push(Inst::Jmp(at));
          let out = prog.len();
          split(prog, at, at + 1, out);
        }
        Some(max) => {
          let mut holes = Vec::new();
          for _ in *min..*max {
            let at = prog.len();
            prog.push(Inst::Jmp(0));
            holes.push(at);
            emit(node, prog, multiline)?;
          }
          let out = prog.len();
          for at in holes {
            split(prog, at, at + 1, out);
          }
        }
      }
    }
  }
  Ok(())
}

impl Regex {
  /// Compile a JS regex source (no flags but multiline).
  pub(crate) fn new(src: &str, multiline: bool) -> Result<Self, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut parser = Parser {
      src: &chars,
      pos: 0,
    };
    let node = parser.alt()?;
    if parser.pos != chars.len() {
      return Err(format!("unbalanced `)` at {}", parser.pos));
    }
    let mut prog = Vec::new();
    emit(&node, &mut prog, multiline)?;
    prog.push(Inst::Match);
    let at_start = !multiline && chars.first() == Some(&'^') && !src.contains('|');
    Ok(Regex {
      prog,
      multiline,
      at_start,
    })
  }

  /// End of the first-priority match starting exactly at `pos`, if any.
  pub(crate) fn match_at(&self, text: &[char], pos: usize) -> Option<usize> {
    if self.at_start && pos != 0 {
      return None;
    }
    let mut stack: Vec<(usize, usize)> = vec![(0, pos)];
    'thread: while let Some((mut pc, mut at)) = stack.pop() {
      loop {
        match &self.prog[pc] {
          Inst::Match => return Some(at),
          Inst::Char(c) => {
            if text.get(at) != Some(c) {
              continue 'thread;
            }
            at += 1;
          }
          Inst::Class(class) => match text.get(at) {
            Some(&c) if class.matches(c) => at += 1,
            _ => continue 'thread,
          },
          Inst::Any => match text.get(at) {
            Some(&c) if !is_line_terminator(c) => at += 1,
            _ => continue 'thread,
          },
          Inst::Bol => {
            let ok = at == 0 || (self.multiline && is_line_terminator(text[at - 1]));
            if !ok {
              continue 'thread;
            }
          }
          Inst::Eol => {
            let ok = at == text.len() || (self.multiline && is_line_terminator(text[at]));
            if !ok {
              continue 'thread;
            }
          }
          Inst::WordBoundary(want) => {
            let before = at > 0 && is_word(text[at - 1]);
            let after = text.get(at).is_some_and(|&c| is_word(c));
            if (before != after) != *want {
              continue 'thread;
            }
          }
          Inst::Look(sub, positive) => {
            if sub.match_at(text, at).is_some() != *positive {
              continue 'thread;
            }
          }
          Inst::Split(first, second) => {
            stack.push((*second, at));
            pc = *first;
            continue;
          }
          Inst::Jmp(to) => {
            pc = *to;
            continue;
          }
        }
        pc += 1;
      }
    }
    None
  }

  /// Whether the pattern matches anywhere in `text`.
  pub(crate) fn is_match(&self, text: &[char]) -> bool {
    self.find_from(text, 0).is_some()
  }

  /// The first match at or after `from`, as (start, end).
  fn find_from(&self, text: &[char], from: usize) -> Option<(usize, usize)> {
    let last = if self.at_start { 0 } else { text.len() };
    (from..=last).find_map(|s| self.match_at(text, s).map(|e| (s, e)))
  }

  /// Number of non-overlapping matches (JS `allMatches(...).length`).
  pub(crate) fn count(&self, text: &[char]) -> usize {
    let mut n = 0;
    let mut from = 0;
    while from <= text.len() {
      let Some((s, e)) = self.find_from(text, from) else {
        break;
      };
      n += 1;
      from = if e > s { e } else { s + 1 };
    }
    n
  }
}

#[cfg(test)]
mod tests {
  use super::Regex;

  fn m(src: &str, text: &str) -> Option<usize> {
    let t: Vec<char> = text.chars().collect();
    Regex::new(src, false).unwrap().match_at(&t, 0)
  }

  #[test]
  fn js_priority_and_lookahead() {
    assert_eq!(m(r"a|ab", "ab"), Some(1));
    assert_eq!(m(r"a*?b", "aab"), Some(3));
    assert_eq!(m(r"[A-Za-z_]\w*(?=\s*:)", "key : 1"), Some(3));
    assert_eq!(m(r"[A-Za-z_]\w*(?=\s*:)", "key = 1"), None);
    assert_eq!(m(r"\w+(?=\s*:(?!=))", "x := 1"), None);
    assert_eq!(m(r"<!--[\s\S]*?(?:-->|$)", "<!-- a\nb --> c"), Some(12));
    assert_eq!(m(r"#{1,6}[^\n]*", "### t\nx"), Some(5));
    assert_eq!(m(r"\$\((?:[^(]|$)", "$("), Some(2));
  }

  #[test]
  fn line_anchors_follow_js() {
    let t: Vec<char> = "a\r\n---\n".chars().collect();
    assert!(Regex::new(r"^---\s*$", true).unwrap().is_match(&t));
    assert!(!Regex::new(r"^---$", false).unwrap().is_match(&t));
    let y: Vec<char> = "a: 1\nb: 2\n".chars().collect();
    assert_eq!(
      Regex::new(r"^[\w.-]+:(?:[ \t]|$)", true).unwrap().count(&y),
      2
    );
  }

  #[test]
  fn a_long_line_does_not_recurse() {
    let line: Vec<char> = "x".repeat(200_000).chars().collect();
    assert_eq!(
      Regex::new(r"[^\n]*", false).unwrap().match_at(&line, 0),
      Some(200_000)
    );
  }
}
