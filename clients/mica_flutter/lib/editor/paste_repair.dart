// Paste-only repairs for Markdown that LLM chat UIs emit (a second
// equal-length fence around a code block; Python labelled ```bash). They run
// on the pasted TEXT before it reaches the engine and are deliberately not
// part of the grammar: file import, MCP writes and the server parse strictly.
// Paste heuristics, not grammar, so they stay here; the language signatures
// `retagMislabeledFences` asks for come from the engine (via `highlight.dart`).
// docs/single-source/02-architecture.md §2.1, decision D7.
library;

import '../markdown/markdown_codec.dart';
import 'highlight.dart' show canonicalCodeLanguage, strongLanguageSignature;

/// Correct a fence whose language label the code itself contradicts.
///
/// ChatGPT (and anything else built on highlight.js auto-detect) labels an
/// unlabelled code block by guessing, and `bash` is that guess's favourite
/// fallback — so Python arrives as ```` ```bash ```` and renders in the wrong
/// colours. We honour an explicit label over our own detection, which is right
/// in general and exactly wrong here: the label isn't the author's, it's
/// another tool's bad guess.
///
/// So: trust the label, unless the code *plainly* says otherwise. Only a
/// [strongLanguageSignature] — structural syntax, never a suggestive word —
/// overrules it, and an unrecognised label is always left alone.
///
/// PASTE-ONLY, for two reasons. It must not touch stored documents or file
/// import, where the fence is the author's own and round-tripping it is an
/// invariant (see [unwrapNestedFences]). And it must not reach
/// [resolveCodeLanguage], where a label picked from the dropdown is a
/// deliberate human choice — overruling *that* would be worse than the bug.
String retagMislabeledFences(String md) {
  final lines = md.split('\n');
  final fenceRe = RegExp(r'^( {0,3})(`{3,}|~{3,})\s*([\w+#.-]*)\s*$');
  var i = 0;
  while (i < lines.length) {
    final m = fenceRe.firstMatch(lines[i]);
    if (m == null) {
      i++;
      continue;
    }
    final indent = m.group(1)!;
    final fence = m.group(2)!;
    final label = m.group(3)!;
    // Find this block's closer, and collect the code between.
    var j = i + 1;
    final body = <String>[];
    while (j < lines.length) {
      final c = fenceRe.firstMatch(lines[j]);
      if (c != null &&
          c.group(3)!.isEmpty &&
          c.group(2)![0] == fence[0] &&
          c.group(2)!.length >= fence.length) {
        break;
      }
      body.add(lines[j]);
      j++;
    }
    if (label.isNotEmpty) {
      final claimed = canonicalCodeLanguage(label);
      final strong = strongLanguageSignature(body.join('\n'));
      if (strong != null && strong != claimed) {
        lines[i] = '$indent$fence$strong';
      }
    }
    i = j + 1; // resume after the closer — never rescan a block's contents
  }
  return lines.join('\n');
}

/// Unwrap the LLM "double-fence" wrapper (ChatGPT/Codex wrap a code or mermaid
/// block in a SECOND equal-length fence).
///
/// PASTE-ONLY: applied to pasted text before the engine parses it, never to
/// stored documents or file import — the core parser stays strict CommonMark.
/// Precise by construction: it fires only when a BARE fence opener (not while
/// inside a code block) is immediately followed by an equal-marker, equal-length
/// fence. A legit `````\n```mermaid` (4-outer/3-inner) has different lengths and
/// is left alone; two adjacent independent code blocks are distinguished by
/// tracking open/close state.
String unwrapNestedFences(String md) {
  final lines = md.split('\n');
  final fenceRe = RegExp(r'^( {0,3})(`{3,}|~{3,})([^`]*)$');
  ({String ch, int len})? closeOf(String line) {
    final m = fenceRe.firstMatch(line);
    if (m == null || m.group(3)!.trim().isNotEmpty) return null; // closers bare
    return (ch: m.group(2)![0], len: m.group(2)!.length);
  }

  final drop = <int>{};
  ({String ch, int len})? open; // currently-open fence, null = not in code
  var i = 0;
  while (i < lines.length) {
    final m = fenceRe.firstMatch(lines[i]);
    if (m == null) {
      i++;
      continue;
    }
    final ch = m.group(2)![0];
    final len = m.group(2)!.length;
    final bare = m.group(3)!.trim().isEmpty;
    if (open != null) {
      // Inside a code block: only a bare, same-char, long-enough fence closes.
      if (bare && ch == open.ch && len >= open.len) open = null;
      i++;
      continue;
    }
    // Not in code → lines[i] is an OPENER. A bare opener immediately followed
    // by an equal fence is the LLM wrapper (a real nested block would need a
    // longer outer fence).
    if (bare && i + 1 < lines.length) {
      final inner = fenceRe.firstMatch(lines[i + 1]);
      if (inner != null &&
          inner.group(2)![0] == ch &&
          inner.group(2)!.length == len) {
        final innerLen = inner.group(2)!.length;
        var close = -1;
        for (var j = i + 2; j < lines.length; j++) {
          final f = closeOf(lines[j]);
          if (f != null && f.ch == ch && f.len >= innerLen) {
            close = j;
            break;
          }
        }
        if (close != -1) {
          drop.add(i); // redundant outer opener
          // The redundant outer closer, if present: the next non-blank line
          // after the inner close, when it is another equal bare fence.
          for (var j = close + 1; j < lines.length; j++) {
            if (lines[j].trim().isEmpty) continue;
            final f = closeOf(lines[j]);
            if (f != null && f.ch == ch && f.len == len) drop.add(j);
            break;
          }
          i = close + 1; // inner block content already accounted for
          continue;
        }
      }
    }
    open = (ch: ch, len: len); // ordinary opener
    i++;
  }
  if (drop.isEmpty) return md;
  final out = <String>[
    for (var k = 0; k < lines.length; k++)
      if (!drop.contains(k)) lines[k],
  ];
  return out.join('\n');
}

/// Whether a pasted SINGLE line of plain text should land with the marks the
/// engine parsed out of it, rather than as literal characters.
///
/// Not syntax — the engine did the parsing (`markdownCodec.parseInline`); this
/// only decides whether to trust the result for a one-line paste, where the
/// text is as likely to be prose that happens to contain an asterisk as it is
/// Markdown. A sentence copied from a Markdown source or an LLM answer
/// (`路径 \`PredictedMediaLifeLeftPercent\` 的方向`, #185) arrived with its
/// backticks showing, because one line used to insert literally, always.
///
/// Trusted: marks whose delimiters prose does not produce by accident — code
/// spans, `**bold**`, `~~strike~~`, `[links](…)`, formulas. Not trusted: a lone
/// `*…*` / `_…_` italic (`2*3*4`, `*args`-ish fragments), and anything the
/// line would need raw HTML or a footnote definition to mean. One untrusted
/// mark keeps the WHOLE line literal: the parsed text has already lost its
/// delimiters, so it cannot be half-applied.
bool singleLinePasteKeepsMarks(Iterable<String> markTypes) {
  const trusted = {'code', 'bold', 'strike', 'link', 'math'};
  var any = false;
  for (final type in markTypes) {
    if (!trusted.contains(type)) return false;
    any = true;
  }
  return any;
}

/// The text and marks a single pasted line of plain text should land with, or
/// null to paste it literally. The engine's inline parse when
/// [singleLinePasteKeepsMarks] trusts it, else the formula-only parse (so
/// "see $x$ here" still renders its formula inline), else null. Also null with
/// no engine (Web without its wasm), where every paste is literal (02 §5.2).
InlineText? singleLinePasteInline(String line) {
  final parsed = withEngineOrNull(() => markdownCodec.parseInline(line));
  if (parsed != null &&
      singleLinePasteKeepsMarks(parsed.marks.map((m) => m.type))) {
    return parsed;
  }
  final math = withEngineOrNull(() => markdownCodec.parseInlineMath(line));
  return math != null && math.marks.isNotEmpty ? math : null;
}
