//! Writes spike/golden/golden.json from the CURRENT core, before any P0
//! optimisation lands. `tests/equivalence.rs` then refuses any byte that moves.
//!
//! Run: cargo run --manifest-path spike/md-wasm-p0/Cargo.toml --example gen_golden

use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let inputs = manifest.join("../inputs");
    let golden_dir = manifest.join("../golden");
    std::fs::create_dir_all(&golden_dir).expect("create golden dir");

    let mut entries = Vec::new();
    let mut names: Vec<PathBuf> = std::fs::read_dir(&inputs)
        .expect("read inputs")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    names.sort();
    for path in names {
        let md = std::fs::read_to_string(&path).expect("read input");
        let json = mica_md::wire::parse_body_json(&md, false).expect("parse");
        let blocks: Vec<mica_md::wire::BlockJson> =
            serde_json::from_str(&json).expect("decode");
        let marks: usize = blocks
            .iter()
            .map(|b| {
                b.data
                    .get("marks")
                    .and_then(|m| m.as_array())
                    .map_or(0, |a| a.len())
            })
            .sum();
        let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
        entries.push(format!(
            "  \"{}\": {{\"sha256\": \"{}\", \"blocks\": {}, \"marks\": {}, \"json_bytes\": {}}}",
            path.file_stem().unwrap().to_string_lossy(),
            hash,
            blocks.len(),
            marks,
            json.len()
        ));
    }
    let body = format!("{{\n{}\n}}\n", entries.join(",\n"));
    std::fs::write(golden_dir.join("golden.json"), body).expect("write golden");
    println!("wrote {} entries", entries.len());
}
