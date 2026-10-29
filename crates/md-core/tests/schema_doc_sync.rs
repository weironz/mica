//! SCHEMA.md and `schema.rs` cannot drift apart: this parses the markdown
//! tables and asserts every row matches the constants. A rename in either
//! place fails here first, before a release can ship half-updated docs.

use mica_md::schema;

/// Rows of the markdown table in the section `## {heading}`: the first
/// backticked cell of each row, split on ` /` for shared rows.
fn table_first_cells(markdown: &str, heading: &str) -> Vec<String> {
  let section = markdown
    .split("## ")
    .find(|s| s.starts_with(heading))
    .unwrap_or_else(|| panic!("no section '{heading}' in SCHEMA.md"));
  section
    .lines()
    .skip_while(|l| !l.starts_with('|'))
    .skip(2) // header + separator
    .take_while(|l| l.starts_with('|'))
    .filter_map(|l| {
      // First cell between pipes, backticks stripped.
      let cells: Vec<&str> = l.split('|').collect();
      cells.get(1).map(|c| c.trim().trim_matches('`').to_string())
    })
    .flat_map(|c| {
      // `widths` / `width` share a cell.
      c.split(" / ")
        .map(|s| s.trim_matches('`').to_string())
        .collect::<Vec<_>>()
    })
    .filter(|c| !c.is_empty())
    .collect()
}

#[test]
fn block_kinds_table_matches_constants() {
  let md = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/SCHEMA.md")).unwrap();
  let rows = table_first_cells(&md, "Block kinds");
  let mut documented: Vec<&str> = rows
    .iter()
    .map(|s| s.as_str())
    .filter(|s| !s.contains(" = "))
    .collect();
  documented.sort_unstable();
  let mut actual = schema::kinds::ALL.to_vec();
  actual.sort_unstable();
  assert_eq!(documented, actual, "SCHEMA.md kinds table out of sync");
}

#[test]
fn mark_types_table_matches_constants() {
  let md = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/SCHEMA.md")).unwrap();
  let section = md
    .split("## Mark types")
    .nth(1)
    .expect("mark types section");
  // The list is one prose paragraph; every backticked token that names a mark
  // type must be exactly the constants (prose mentions like `href` are
  // filtered by membership).
  let mut documented: Vec<&str> = section
    .lines()
    .take_while(|l| !l.starts_with('#'))
    .flat_map(|l| l.split('`').skip(1).step_by(2).collect::<Vec<_>>())
    .filter(|t| schema::marks::ALL.contains(t))
    .collect();
  documented.sort_unstable();
  documented.dedup();
  let mut actual = schema::marks::ALL.to_vec();
  actual.sort_unstable();
  assert_eq!(
    documented, actual,
    "SCHEMA.md mark list out of sync with schema::marks::ALL"
  );
}

#[test]
fn data_keys_table_matches_constants() {
  let md = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/SCHEMA.md")).unwrap();
  let rows = table_first_cells(&md, "Data keys");
  let mut documented: Vec<&str> = rows.iter().map(|s| s.as_str()).collect();
  documented.sort_unstable();
  documented.dedup();
  let mut actual = schema::data_keys::ALL.to_vec();
  actual.sort_unstable();
  assert_eq!(documented, actual, "SCHEMA.md data-key table out of sync");
}

#[test]
fn aliases_are_documented() {
  let md = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/SCHEMA.md")).unwrap();
  for (alias, _) in schema::kinds::ALIASES {
    assert!(
      md.contains(&format!("`{alias}` =")),
      "alias {alias} missing from SCHEMA.md"
    );
  }
}
