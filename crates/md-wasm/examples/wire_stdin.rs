//! Host counterpart for byte-level checks of a served Web wire result.
use std::io::Read;
fn main() {
  let mut input = String::new();
  std::io::stdin().read_to_string(&mut input).unwrap();
  print!("{}", mica_md::wire::parse_body_json(&input, false).unwrap());
}
