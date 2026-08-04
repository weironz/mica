//! One-off repair: documents where a URL-shaped link LABEL was left as literal
//! Markdown source.
//!
//! Copying a link out of Mica and pasting it back produced `[url](url)` — a link
//! whose label is itself a URL — and the inline parser refused it as a nested
//! link, storing the literal source as block TEXT with a bogus autolink over the
//! parenthesised half. The parser is fixed as of 0.13.12; this repairs what the
//! broken one already wrote.
//!
//! ```text
//! DATABASE_URL=postgres://... cargo run -p mica-app-core --example repair_url_labels
//! DATABASE_URL=postgres://... cargo run -p mica-app-core --example repair_url_labels -- --apply
//! ```
//!
//! **Why not a regex unwrap.** The obvious `[X](Y)` -> `X` is wrong: label and
//! href genuinely differ in this corpus (`[https://ollama.com/](https://blog…/target?url=…)`),
//! so unwrapping to the label would silently rewrite where those links POINT.
//! The fixed parser decides instead — the same code that will parse the text if
//! a human retypes it.
//!
//! **Why whole-block re-parse, and why it must be gated.** Stored text is clean
//! plain text with marks in `data`, so re-parsing a block that carries real
//! formatting would drop it (`**bold**` is not in the text — the mark is). A
//! block is only rewritten when every mark it has is one of the bogus autolinks
//! INSIDE the literal spans; anything else is skipped and reported, not guessed
//! at. In practice that gate costs nothing: a block corrupted this way came from
//! pasting Markdown source, so its other formatting is literal too.
//!
//! Deliberately an example, not a bin: it runs once, against one database, and
//! has no business being in a shipped binary.

use mica_app_core::sync::content_text_from_doc;
use mica_core::{marks_from_data, Block, Mark, MicaDoc};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

/// One `[LABEL](HREF)` run in a block's text, as BYTE offsets into it.
struct Literal {
    start: usize,
    end: usize,
}

/// Every `[LABEL](HREF)` whose LABEL is itself URL-shaped.
///
/// Hand-rolled rather than a regex dependency, and narrow on purpose: a label
/// that is NOT a URL never hit the bug (`[docs](https://x)` always parsed), so
/// widening this would put untouched documents in the blast radius.
fn literals(text: &str) -> Vec<Literal> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'[' {
            i += 1;
            continue;
        }
        let label_start = i + 1;
        let rest = &text[label_start..];
        if !(rest.starts_with("http://") || rest.starts_with("https://") || rest.starts_with("www."))
        {
            i += 1;
            continue;
        }
        // `](` must follow the label, and the label may not contain `]`.
        let Some(close) = rest.find(']') else { break };
        if !rest[close..].starts_with("](") {
            i += 1;
            continue;
        }
        let href_start = label_start + close + 2;
        let Some(paren) = text[href_start..].find(')') else {
            i += 1;
            continue;
        };
        let end = href_start + paren + 1;
        out.push(Literal { start: i, end });
        i = end;
    }
    out
}

/// UTF-16 code units in `s` — the unit yrs marks are measured in
/// (`OffsetKind::Utf16`), so mark offsets are only comparable in these.
fn u16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

fn marks_of(block: &Block) -> Vec<Mark> {
    marks_from_data(&block.data)
}

/// True when every mark sits fully inside one of the literal spans — i.e. the
/// block carries nothing but the damage, so a whole-text re-parse loses nothing.
fn only_bogus_marks(text: &str, marks: &[Mark], lits: &[Literal]) -> bool {
    let spans: Vec<(u32, u32)> = lits
        .iter()
        .map(|l| (u16_len(&text[..l.start]), u16_len(&text[..l.end])))
        .collect();
    marks
        .iter()
        .all(|m| spans.iter().any(|(s, e)| m.start >= *s && m.end <= *e))
}

/// The fixed parser's verdict on this text: `(text, marks)`, or `None` when it
/// is not a single block (the caller must not guess at multi-block results).
fn reparse(text: &str) -> Option<(String, Vec<Mark>)> {
    let payload = mica_markdown::import_markdown(text, "repair-root");
    let root = payload.root_block_id.clone();
    let mut content = payload.blocks.into_iter().filter(|b| b.id != root);
    let block = content.next()?;
    if content.next().is_some() {
        return None;
    }
    let marks = marks_from_data(&block.data);
    Some((block.text, marks))
}

/// Repair by SPLICING only the damaged spans, for blocks that carry marks
/// elsewhere and so cannot survive a whole-text re-parse.
///
/// The parser still decides — it just runs on the damaged FRAGMENT instead of
/// the whole block, and the rest of the text is carried across untouched with
/// its marks shifted by the length the splice changed. Returns `None` rather
/// than guessing whenever the mapping would be a guess: a fragment the parser
/// does not turn into a single block, or a mark that straddles a span boundary
/// (no honest place to put its endpoint).
fn splice(text: &str, marks: &[Mark], lits: &[Literal]) -> Result<(String, Vec<Mark>), &'static str> {
    let mut new_text = String::new();
    let mut out: Vec<Mark> = Vec::new();
    // (old_start, old_end, new_start, new_end), all UTF-16.
    let mut map: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut cursor = 0usize;
    for l in lits {
        let frag = &text[l.start..l.end];
        let (frag_text, frag_marks) = reparse(frag).ok_or("fragment is not one block")?;
        if frag_text == frag {
            continue;
        }
        new_text.push_str(&text[cursor..l.start]);
        let (old_s, old_e) = (u16_len(&text[..l.start]), u16_len(&text[..l.end]));
        let new_s = u16_len(&new_text);
        new_text.push_str(&frag_text);
        let new_e = u16_len(&new_text);
        for m in frag_marks {
            out.push(Mark {
                start: m.start + new_s,
                end: m.end + new_s,
                ..m
            });
        }
        map.push((old_s, old_e, new_s, new_e));
        cursor = l.end;
    }
    if map.is_empty() {
        return Err("parser leaves the fragment unchanged");
    }
    new_text.push_str(&text[cursor..]);

    for m in marks {
        // Fully inside a replaced span: this IS the bogus autolink, and the
        // fragment re-parse already produced whatever replaces it.
        if map.iter().any(|(os, oe, ..)| m.start >= *os && m.end <= *oe) {
            continue;
        }
        if map
            .iter()
            .any(|(os, oe, ..)| m.start < *oe && m.end > *os)
        {
            return Err("a mark straddles the damaged span");
        }
        let shift = |p: u32| -> u32 {
            let d: i64 = map
                .iter()
                .filter(|(_, oe, ..)| p >= *oe)
                .map(|(os, oe, ns, ne)| (*ne as i64 - *ns as i64) - (*oe as i64 - *os as i64))
                .sum();
            (p as i64 + d) as u32
        };
        out.push(Mark {
            start: shift(m.start),
            end: shift(m.end),
            ..m.clone()
        });
    }
    out.sort_by_key(|m| (m.start, m.end));
    Ok((new_text, out))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let apply = std::env::args().any(|a| a == "--apply");
    let url = std::env::var("DATABASE_URL").map_err(|_| "set DATABASE_URL")?;
    let pool = PgPoolOptions::new().max_connections(4).connect(&url).await?;

    // Same narrow signature as the eligibility test below, pushed into SQL so
    // the scan reads a few hundred rows instead of every document.
    let rows: Vec<(Uuid, Vec<u8>)> = sqlx::query_as(
        "SELECT document_id, state FROM document_yrs_base
         WHERE content_text ~ '[[](https?://|www[.])[^]]*[]][(]'
         ORDER BY document_id",
    )
    .fetch_all(&pool)
    .await?;
    println!("candidate documents: {}", rows.len());

    let (mut docs_changed, mut blocks_changed, mut links_fixed) = (0usize, 0usize, 0usize);
    let mut skipped: Vec<String> = Vec::new();
    let mut samples: Vec<String> = Vec::new();

    for (doc_id, state) in rows {
        let mut doc = match MicaDoc::from_update(&state) {
            Ok(d) => d,
            Err(e) => {
                skipped.push(format!("{doc_id}: undecodable base ({e})"));
                continue;
            }
        };
        let mut edits: Vec<(String, String, Vec<Mark>)> = Vec::new();
        for block in doc.to_blocks() {
            // Code and atomic blocks hold source, not prose: `[url](url)` inside
            // them is CONTENT and re-parsing it would corrupt what it displays.
            if block.kind.contains("code") || block.kind == "math" || block.kind == "mermaid" {
                continue;
            }
            let lits = literals(&block.text);
            if lits.is_empty() {
                continue;
            }
            let marks = marks_of(&block);
            // Whole-text re-parse where the block carries nothing but the
            // damage (it was pasted Markdown source, so ALL of it is literal);
            // span splice where it carries real formatting a re-parse would
            // drop. Both let the fixed parser decide the link.
            let repaired = if only_bogus_marks(&block.text, &marks, &lits) {
                reparse(&block.text).ok_or("whole block is not one block")
            } else {
                splice(&block.text, &marks, &lits)
            };
            let (new_text, new_marks) = match repaired {
                Ok(v) => v,
                Err(why) => {
                    skipped.push(format!("{doc_id} block {}: {why}", block.id));
                    continue;
                }
            };
            if new_text == block.text {
                continue;
            }
            if samples.len() < 6 {
                let href = new_marks
                    .first()
                    .and_then(|m| m.href.clone())
                    .unwrap_or_default();
                samples.push(format!(
                    "  {} -> {} [href={}]",
                    block.text.chars().take(70).collect::<String>(),
                    new_text.chars().take(50).collect::<String>(),
                    href.chars().take(50).collect::<String>()
                ));
            }
            links_fixed += lits.len();
            edits.push((block.id, new_text, new_marks));
        }
        if edits.is_empty() {
            continue;
        }
        docs_changed += 1;
        blocks_changed += edits.len();
        if !apply {
            continue;
        }
        for (id, text, marks) in &edits {
            doc.set_block_text(id, text, marks);
        }
        // The new state is the old history PLUS these ops, not a replacement, so
        // a client still holding the old doc converges on reconnect instead of
        // landing in a parallel CRDT universe. `base_rid` is untouched: it marks
        // how far the remote update stream is folded in, and nothing about that
        // changed.
        sqlx::query(
            "UPDATE document_yrs_base
             SET state = $2, state_vector = $3, content_text = $4, updated_at = now()
             WHERE document_id = $1",
        )
        .bind(doc_id)
        .bind(doc.encode_state())
        .bind(doc.state_vector())
        .bind(content_text_from_doc(&doc))
        .execute(&pool)
        .await?;
    }

    println!("\nsamples:");
    for s in &samples {
        println!("{s}");
    }
    if !skipped.is_empty() {
        println!("\nskipped ({}):", skipped.len());
        for s in skipped.iter().take(25) {
            println!("  {s}");
        }
    }
    println!(
        "\n{} documents, {} blocks, {} links {}",
        docs_changed,
        blocks_changed,
        links_fixed,
        if apply {
            "REPAIRED"
        } else {
            "would change (dry run; pass --apply)"
        }
    );
    Ok(())
}
