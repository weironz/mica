use mica_markdown::{export_markdown, import_markdown};

#[test]
fn ordered_list_numbers_continue_in_export() {
  let src = "1. first\n2. second\n3. third\n";
  let out = export_markdown(&import_markdown(src, "root")).unwrap();
  assert_eq!(out, src.trim_end());
}

#[test]
fn ordered_list_after_non_list_block_resets() {
  let src = "1. first\n2. second\n\nparagraph\n\n1. first again\n";
  let out = export_markdown(&import_markdown(src, "root")).unwrap();
  assert_eq!(out, src.trim_end());
}

#[test]
fn nested_ordered_lists_number_and_reset_by_level() {
  let src = "1. outer one\n    1. inner one\n    2. inner two\n2. outer two\n";
  let out = export_markdown(&import_markdown(src, "root")).unwrap();
  assert_eq!(out, src.trim_end());
}

#[test]
fn bullet_between_ordered_lists_resets_the_counter() {
  let src = "1. one\n- bullet\n1. two\n";
  let out = export_markdown(&import_markdown(src, "root")).unwrap();
  assert_eq!(out, src.trim_end());
}

#[test]
fn quote_groups_keep_ordered_counters_independent() {
  let src = "> 1. one\n> 2. two\n\n> 1. three\n";
  let out = export_markdown(&import_markdown(src, "root")).unwrap();
  assert_eq!(out, src.trim_end());
}

#[test]
fn explicit_start_restarts_the_ordered_counter() {
  let src = "3. three\n4. four\n\nparagraph\n\n7. seven\n8. eight\n";
  let out = export_markdown(&import_markdown(src, "root")).unwrap();
  assert_eq!(out, src.trim_end());
}

#[test]
fn later_source_numbers_do_not_restart_a_commonmark_list() {
  let src = "1. one\n3. two\n4. three";
  let out = export_markdown(&import_markdown(src, "root")).unwrap();
  assert_eq!(out, "1. one\n2. two\n3. three");
}

#[test]
fn numbered_export_preserves_html_for_container_children_and_wide_markers() {
  for src in [
    "100. outer\n     1. inner\n     2. inner again\n101. next",
    "1. first\n\n    child paragraph\n\n2. second",
    "1. outer\n    - bullet\n    1. nested one\n    2. nested two\n2. outer two",
    "999999999. first\n1. second",
  ] {
    let original = import_markdown(src, "root");
    let md = export_markdown(&original).unwrap();
    let after = import_markdown(&md, "root");
    assert_eq!(
      mica_markdown::export_html(&after).unwrap(),
      mica_markdown::export_html(&original).unwrap(),
      "{src}\nexported:\n{md}"
    );
  }
}
