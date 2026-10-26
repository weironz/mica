//! Host guards for the P0 experiment.
//!
//! 1. The optimized core must produce byte-identical §3.3 JSON for every
//!    benchmark input (golden generated from the pre-optimisation commit).
//! 2. UUID and Sequential parses of the same input must differ only in ids.
//! 3. The wire format sorts `data.marks` by (start, end, type).

use serde_json::Value;
use sha2::Digest;
use std::path::PathBuf;

fn inputs() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../inputs");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read inputs")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            (
                p.file_stem().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).expect("read input"),
            )
        })
        .collect()
}

#[test]
fn optimized_core_matches_p0_golden() {
    let golden: Value = serde_json::from_str(
        &std::fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../golden/golden.json"),
        )
        .expect("golden.json"),
    )
    .expect("parse golden");
    for (name, md) in inputs() {
        let json = mica_md::wire::parse_body_json(&md, false).expect("parse");
        let hash = format!("{:x}", sha2::Sha256::digest(json.as_bytes()));
        let want = golden
            .get(&name)
            .and_then(|v| v.get("sha256"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| panic!("no golden entry for {name}"));
        assert_eq!(hash, want, "wire JSON changed for {name}");
    }
}

#[cfg(feature = "uuid")]
#[test]
fn uuid_and_sequential_differ_only_in_ids() {
    use mica_md::{BlockIds, ParseMode, ParseOptions};
    for (name, md) in inputs().into_iter().take(3) {
        let sequential = mica_md::parse(
            &md,
            &ParseOptions {
                mode: ParseMode::Document,
                root_id: "root".to_string(),
                ids: BlockIds::Sequential {
                    prefix: "b".to_string(),
                },
            },
        );
        let uuid = mica_md::parse(
            &md,
            &ParseOptions {
                mode: ParseMode::Document,
                root_id: "root".to_string(),
                ids: BlockIds::Uuid,
            },
        );
        assert_eq!(sequential.blocks.len(), uuid.blocks.len(), "{name}");
        for (a, b) in sequential.blocks.iter().zip(uuid.blocks.iter()) {
            assert_eq!(a.kind, b.kind, "{name}");
            assert_eq!(a.text, b.text, "{name}");
            assert_eq!(a.data, b.data, "{name}");
            assert_eq!(a.children.len(), b.children.len(), "{name}");
            assert!(!a.id.is_empty(), "{name}");
            assert!(!b.id.is_empty(), "{name}");
        }
    }
}

#[test]
fn wire_marks_are_sorted() {
    for (name, md) in inputs() {
        let json = mica_md::wire::parse_body_json(&md, false).expect("parse");
        let blocks: Vec<mica_md::wire::BlockJson> = serde_json::from_str(&json).expect("decode");
        for block in blocks {
            let Some(marks) = block.data.get("marks").and_then(|m| m.as_array()) else {
                continue;
            };
            let keys: Vec<(u64, u64, String)> = marks
                .iter()
                .map(|m| {
                    (
                        m.get("start").and_then(Value::as_u64).unwrap_or(0),
                        m.get("end").and_then(Value::as_u64).unwrap_or(0),
                        m.get("type")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    )
                })
                .collect();
            let mut sorted = keys.clone();
            sorted.sort();
            assert_eq!(keys, sorted, "{name}: marks out of order");
        }
    }
}
