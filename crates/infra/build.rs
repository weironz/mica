fn main() {
  // sqlx::migrate! cannot discover newly added files through its old expansion.
  println!("cargo:rerun-if-changed=../../migrations");
}
