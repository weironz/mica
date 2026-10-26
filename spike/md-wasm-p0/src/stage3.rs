//! SPIKE ONLY: owned Block -> BlockJson projection mirror of wire.rs.
//! Production helper is private; do not change the core for instrumentation.
//! Host tests and every measured wasm call compare the entire JSON byte string
//! against the real wire function before accepting a timing result.
use mica_md::{Block, wire::BlockJson};
use serde_json::Value;

fn normalize_data(mut data: Value) -> Value {
    if data.is_null() {
        return serde_json::json!({});
    }
    if let Some(marks) = data.get_mut("marks").and_then(Value::as_array_mut) {
        marks.sort_by(|a, b| {
            let key = |m: &Value| (
                m.get("start").and_then(Value::as_u64).unwrap_or(0),
                m.get("end").and_then(Value::as_u64).unwrap_or(0),
            );
            let (a_start, a_end) = key(a);
            let (b_start, b_end) = key(b);
            a_start.cmp(&b_start).then(a_end.cmp(&b_end)).then_with(|| {
                a.get("type").and_then(Value::as_str).unwrap_or("")
                    .cmp(b.get("type").and_then(Value::as_str).unwrap_or(""))
            })
        });
    }
    data
}

pub fn project(blocks: Vec<Block>) -> Vec<BlockJson> {
    blocks.into_iter().map(|block| BlockJson {
        kind: block.kind,
        text: block.text,
        data: normalize_data(block.data),
    }).collect()
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::*;
    use std::hint::black_box;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = performance, js_name = now)]
        fn now() -> f64;
    }

    /// One parse per call prepares the measured body; the real wire reference
    /// performs a separate parse for validation, also BEFORE either timer.
    /// Deep clone inputs are prepared BEFORE timing. Projection takes ownership,
    /// just as production.
    /// JSON encoding borrows the projected rows and returns owned Strings.
    /// Their full contents are consumed AFTER timers, before returning a small
    /// JSON metrics string to JS. No document JSON crosses the boundary inside
    /// the encoding interval. Neither parse, preparation nor validation is timed.
    #[wasm_bindgen(js_name = stage3Probe)]
    pub fn stage3_probe(markdown: &str, iterations: u32) -> Result<String, JsError> {
        let n = iterations as usize;
        if !(1..=8192).contains(&n) {
            return Err(JsError::new("iterations outside 1..8192"));
        }
        let blocks = mica_md::parse_body(markdown, mica_md::ParseMode::Document);
        let expected = mica_md::wire::parse_body_json(markdown, false)
            .map_err(|e| JsError::new(&e.to_string()))?;
        let prepared: Vec<Vec<Block>> = (0..n).map(|_| blocks.clone()).collect();
        // Allocate only the OUTER vector outside timing. Each inner projection
        // allocation, normalization, mark sort, and dropped id/children is timed.
        let mut projected = Vec::with_capacity(n);
        let map_start = now();
        for body in prepared {
            projected.push(black_box(super::project(black_box(body))));
        }
        let map_end = now();

        let mut outputs = Vec::with_capacity(n);
        let encode_start = now();
        for rows in &projected {
            outputs.push(black_box(serde_json::to_string(black_box(rows))
                .map_err(|e| JsError::new(&e.to_string()))?));
        }
        let encode_end = now();

        // Full output byte equality + full byte checksum (no length-only guard).
        // Output destruction follows validation, also OUTSIDE timed intervals.
        let mut checksum = 0u32;
        for output in &outputs {
            if output.as_bytes() != expected.as_bytes() {
                return Err(JsError::new("stage3 projection differs from real wire JSON"));
            }
            for byte in output.as_bytes() {
                checksum = checksum.wrapping_mul(31).wrapping_add(*byte as u32);
            }
        }
        Ok(serde_json::json!({
            "iterations": n, "blocks": blocks.len(), "jsonBytes": expected.len(),
            "projectionTotalMs": map_end - map_start,
            "encodeTotalMs": encode_end - encode_start,
            "projectionMs": (map_end - map_start) / n as f64,
            "encodeMs": (encode_end - encode_start) / n as f64,
            "wireBytesEqual": true, "checksum": checksum,
            "engineInfo": mica_md::wire::engine_info_json(),
        }).to_string())
    }

    /// Raw performance.now import calls: total loop time / count includes the
    /// call + loop + accumulation. Also return the min nonzero tick quantum.
    /// This is a baseline, NEVER subtracted as an exact correction.
    #[wasm_bindgen(js_name = stage3TimerBaseline)]
    pub fn timer_baseline(iterations: u32) -> String {
        let n = iterations.clamp(2, 100_000);
        let started = now();
        let mut last = started;
        let mut min_tick = f64::INFINITY;
        let mut sink = 0.0;
        for _ in 0..n {
            let current = black_box(now());
            let delta = current - last;
            if delta > 0.0 { min_tick = min_tick.min(delta); }
            sink += current;
            last = current;
        }
        let elapsed = now() - started;
        serde_json::json!({"iterations":n,"totalMs":elapsed,
            "callLoopMs":elapsed / n as f64,
            "minNonzeroTickMs":if min_tick.is_finite(){Some(min_tick)}else{None},
            "sink":sink}).to_string()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn projection_mirror_is_byte_identical_to_real_wire() {
        for md in [
            include_str!("../../inputs/p0_87.md"),
            include_str!("../../inputs/p0_10k.md"),
            include_str!("../../inputs/p0_100k.md"),
            include_str!("../../inputs/s0_87.md"),
            include_str!("../../inputs/s0_10k.md"),
            include_str!("../../inputs/s0_100k.md"),
            include_str!("../../inputs/s0_sample.md"),
        ] {
            let rows = super::project(mica_md::parse_body(md, mica_md::ParseMode::Document));
            assert_eq!(serde_json::to_string(&rows).unwrap(),
                mica_md::wire::parse_body_json(md, false).unwrap());
        }
    }

    #[test]
    fn projection_mirror_preserves_tie_order_and_null_normalization() {
        let marks=serde_json::json!([
            {"start":1,"end":2,"type":"bold","tag":"first"},
            {"start":0,"end":1,"type":"italic"},
            {"start":1,"end":2,"type":"bold","tag":"second"}
        ]);
        let blocks=vec![mica_md::Block::new("a","paragraph"),
            mica_md::Block::new("b","paragraph").with_data(serde_json::json!({"marks":marks}))];
        let rows=super::project(blocks);
        assert_eq!(rows[0].data,serde_json::json!({}));
        assert_eq!(rows[1].data["marks"][0]["start"],0);
        assert_eq!(rows[1].data["marks"][1]["tag"],"first");
        assert_eq!(rows[1].data["marks"][2]["tag"],"second");
    }
}
