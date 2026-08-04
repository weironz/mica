//! One-off repair: ranges like `1~2` that the parser turned into strikethrough.
//!
//! A single `~` used to open and close strikethrough anywhere, so a paragraph
//! with two ranges in it ("1个汉字 ≈ 1~2个 Token，英文…≈ 1~2个 Token") had the
//! two tildes pair up with each other and everything between them struck out.
//! Because this is a WYSIWYG editor the tildes were CONSUMED into the mark —
//! the text on disk reads `12个 Token` and no amount of editing brings them
//! back. The parser is fixed; this puts the tildes back.
//!
//! ```text
//! DATABASE_URL=postgres://... cargo run -p mica-app-core --example repair_tilde_ranges
//! DATABASE_URL=postgres://... cargo run -p mica-app-core --example repair_tilde_ranges -- --apply
//! ```
//!
//! **The signature is the mark, not the text.** A strike mark whose immediate
//! neighbours on BOTH sides are word characters is exactly what the new rule
//! refuses to produce, so it can only have come from this bug — a deliberate
//! `~~struck~~` sits next to spaces or punctuation. That is also why this
//! cannot be a text scan: the tildes are gone, and only the mark's position
//! still says where they were.
//!
//! Deliberately an example, not a bin: it runs once, against one database.

use mica_app_core::sync::content_text_from_doc;
use mica_core::{marks_from_data, Mark, MicaDoc};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

/// Neither whitespace nor punctuation — the same test the parser now applies
/// (`is_word_char` in crates/markdown). ASCII punctuation only: this decides
/// whether to TOUCH a document, so it errs toward leaving things alone.
fn is_word_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_ascii_punctuation()
}

/// The UTF-16 offset `p` as a byte index into `s`, if it lands on a boundary.
fn char_index(s: &str, p: u32) -> Option<usize> {
    let mut u16s = 0u32;
    for (idx, c) in s.char_indices() {
        if u16s == p {
            return Some(idx);
        }
        u16s += c.len_utf16() as u32;
    }
    (u16s == p).then_some(s.len())
}

/// True when this strike mark has word characters on BOTH sides — the shape the
/// fixed parser will no longer produce.
fn is_victim(text: &str, m: &Mark) -> bool {
    if m.ty != "strike" || m.end <= m.start {
        return false;
    }
    match (char_index(text, m.start), char_index(text, m.end)) {
        (Some(s), Some(e)) => {
            text[..s].chars().next_back().is_some_and(is_word_char)
                && text[e..].chars().next().is_some_and(is_word_char)
        }
        _ => false,
    }
}

/// Put the two tildes back around every intraword strike mark in this block.
///
/// `None` when nothing here matches, or when an offset does not land on a
/// character boundary (garbled input — leave it alone rather than guess).
fn restore(text: &str, marks: &[Mark]) -> Option<(String, Vec<Mark>)> {
    let mut victims: Vec<Mark> = marks.iter().filter(|m| is_victim(text, m)).cloned().collect();
    if victims.is_empty() {
        return None;
    }
    // Right to left, so the offsets we have not spliced yet stay valid.
    victims.sort_by_key(|m| std::cmp::Reverse(m.start));

    let mut out = text.to_string();
    let mut kept: Vec<Mark> = marks.iter().filter(|m| !is_victim(text, m)).cloned().collect();
    for v in &victims {
        let s = char_index(&out, v.start)?;
        let e = char_index(&out, v.end)?;
        out.insert(e, '~');
        out.insert(s, '~');
        // Everything at or after the restored tildes moves: +1 past the opener,
        // +2 past the closer. A mark that STARTS where the opener went belongs
        // after it, hence `>=`.
        for m in &mut kept {
            for p in [&mut m.start, &mut m.end] {
                if *p >= v.end {
                    *p += 2;
                } else if *p >= v.start {
                    *p += 1;
                }
            }
        }
    }
    kept.sort_by_key(|m| (m.start, m.end));
    Some((out, kept))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let apply = std::env::args().any(|a| a == "--apply");
    let url = std::env::var("DATABASE_URL").map_err(|_| "set DATABASE_URL")?;
    let pool = PgPoolOptions::new().max_connections(4).connect(&url).await?;

    // No SQL pre-filter is possible: the evidence is a MARK, and marks live in
    // the CRDT blob, not in content_text. So every document gets decoded.
    let rows: Vec<(Uuid, Vec<u8>)> =
        sqlx::query_as("SELECT document_id, state FROM document_yrs_base ORDER BY document_id")
            .fetch_all(&pool)
            .await?;
    println!("documents scanned: {}", rows.len());

    let (mut docs, mut blocks, mut restored) = (0usize, 0usize, 0usize);
    let mut samples: Vec<String> = Vec::new();

    for (doc_id, state) in rows {
        let Ok(mut doc) = MicaDoc::from_update(&state) else {
            continue;
        };
        let mut edits: Vec<(String, String, Vec<Mark>)> = Vec::new();
        for block in doc.to_blocks() {
            if block.kind.contains("code") {
                continue;
            }
            let marks = marks_from_data(&block.data);
            let Some((new_text, new_marks)) = restore(&block.text, &marks) else {
                continue;
            };
            restored += marks.len() - new_marks.len();
            if samples.len() < 6 {
                samples.push(format!(
                    "  {}\n    -> {}",
                    block.text.chars().take(78).collect::<String>(),
                    new_text.chars().take(78).collect::<String>()
                ));
            }
            edits.push((block.id, new_text, new_marks));
        }
        if edits.is_empty() {
            continue;
        }
        docs += 1;
        blocks += edits.len();
        if !apply {
            continue;
        }
        for (id, text, marks) in &edits {
            doc.set_block_text(id, text, marks);
        }
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
    println!(
        "\n{docs} documents, {blocks} blocks, {restored} ranges {}",
        if apply {
            "RESTORED"
        } else {
            "would change (dry run; pass --apply)"
        }
    );
    Ok(())
}
