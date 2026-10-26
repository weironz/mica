use mica_markdown::{export_markdown, import_markdown};

#[test]
fn unclosed_math_keeps_all_following_text() {
  for opener in ["$$", "\\["] {
    let snapshot = import_markdown(&format!("{opener}\nsecret\nmore"), "root");
    let body: Vec<_> = snapshot
      .blocks
      .iter()
      .filter(|b| b.id != snapshot.root_block_id)
      .collect();
    assert_eq!(body.len(), 1, "{opener}: {body:?}");
    assert_eq!(body[0].kind, "paragraph");
    assert_eq!(
      body[0].text,
      format!(
        "{}\nsecret\nmore",
        if opener == "\\[" { "[" } else { opener }
      )
    );
    let exported = export_markdown(&snapshot).unwrap();
    let again = import_markdown(&exported, "root");
    assert_eq!(
      again
        .blocks
        .iter()
        .find(|b| b.id != again.root_block_id && b.kind == "paragraph")
        .unwrap()
        .text,
      body[0].text
    );
  }
}

#[test]
fn closed_math_still_consumes_only_its_own_lines() {
  for (opener, closer) in [("$$", "$$"), ("\\[", "\\]")] {
    let snapshot = import_markdown(&format!("{opener}\nsecret\n{closer}\n\nafter"), "root");
    assert!(
      snapshot
        .blocks
        .iter()
        .any(|b| b.kind == "math_block" && b.text == "secret")
    );
    assert!(
      snapshot
        .blocks
        .iter()
        .any(|b| b.kind == "paragraph" && b.text == "after")
    );
  }
}
