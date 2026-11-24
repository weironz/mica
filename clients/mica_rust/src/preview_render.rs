//! Unified Math (`ratex`) and Mermaid (`merman`) preview renderer.
//!
//! - On native desktop, renders synchronously via `mica-md` (`render-math` + `render-mermaid`)
//!   with an in-memory per-theme cache.
//! - On Web (`wasm32`), `mica-md` is built without `render-math` / `render-mermaid` so the
//!   initial WASM bundle stays lean (<= 4.0 MiB Brotli-11). The first Math or Mermaid block
//!   encountered invokes `window.__micaPreviewRenderMath` / `window.__micaPreviewRenderMermaid`,
//!   which dynamically imports `./mica_preview/{math,mermaid}.js` + `.wasm`, caches the SVG
//!   result, and bumps [`preview_rev`] so reactive `dynamic(...)` views repaint immediately.
//!   Ordinary pages without Math or Mermaid blocks trigger zero `/mica_preview/*` requests.

use std::cell::RefCell;
use std::collections::HashMap;

use mica_md::MermaidHostTheme;
use mica_md::render::MathSvg;
use nsh_core::prelude::{Owner, Signal, signal};

const MAX_CACHE_ENTRIES: usize = 64;

thread_local! {
  static PREVIEW_OWNER: RefCell<Option<Owner>> = const { RefCell::new(None) };
  static PREVIEW_REV: RefCell<Option<Signal<u64>>> = const { RefCell::new(None) };
  static MERMAID_CACHE: RefCell<HashMap<(bool, String), Option<String>>> =
    RefCell::new(HashMap::new());
  static MATH_CACHE: RefCell<HashMap<(bool, String, String), Option<MathSvg>>> =
    RefCell::new(HashMap::new());
  #[cfg(target_arch = "wasm32")]
  static MERMAID_INFLIGHT: RefCell<std::collections::HashSet<(bool, String)>> =
    RefCell::new(std::collections::HashSet::new());
  #[cfg(target_arch = "wasm32")]
  static MATH_INFLIGHT: RefCell<std::collections::HashSet<(bool, String, String)>> =
    RefCell::new(std::collections::HashSet::new());
}

/// Bind the root NSH reactive [`Owner`] once at app startup so asynchronous Web
/// preview tasks run inside NSH's reactive runtime (preventing `no reactive
/// runtime` panics when updating [`preview_rev`] and surviving individual block
/// re-renders while a lazy preview WASM module is downloading).
pub fn init_preview_owner(owner: Owner) {
  PREVIEW_OWNER.with(|cell| {
    *cell.borrow_mut() = Some(owner);
  });
  let _ = preview_rev();
}

#[cfg(target_arch = "wasm32")]
fn spawn_on_preview_runtime<F>(fut: F)
where
  F: std::future::Future<Output = ()> + 'static,
{
  let owner = PREVIEW_OWNER.with(|cell| *cell.borrow());
  if let Some(owner) = owner {
    owner.run(move || {
      nsh_core::prelude::spawn(fut);
    });
  } else {
    nsh_core::prelude::spawn(fut);
  }
}

/// Global reactive counter incremented whenever an asynchronous Web preview
/// finishes rendering. Subscribe via `let _ = preview_render::preview_rev().get();`
/// inside `dynamic(...)` blocks that render Math or Mermaid previews.
pub fn preview_rev() -> Signal<u64> {
  PREVIEW_REV.with(|cell| {
    if let Some(sig) = *cell.borrow() {
      return sig;
    }
    let sig = signal(0u64);
    *cell.borrow_mut() = Some(sig);
    sig
  })
}

#[cfg(target_arch = "wasm32")]
fn bump_preview_rev() {
  PREVIEW_REV.with(|cell| {
    if let Some(sig) = *cell.borrow() {
      sig.set(sig.get_untracked().wrapping_add(1));
    }
  });
}

/// Build the host-theme palette passed to `merman` (native or `mica_preview/mermaid.wasm`).
pub fn mermaid_host_theme(dark_mode: bool) -> MermaidHostTheme {
  MermaidHostTheme {
    dark: dark_mode,
    canvas: if dark_mode { "#18181b" } else { "#ffffff" }.into(),
    surface: if dark_mode { "#27272a" } else { "#f4f4f5" }.into(),
    surface_alt: if dark_mode { "#3f3f46" } else { "#e4e4e7" }.into(),
    text: if dark_mode { "#f4f4f5" } else { "#18181b" }.into(),
    subtle_text: if dark_mode { "#a1a1aa" } else { "#71717a" }.into(),
    border: if dark_mode { "#52525b" } else { "#d4d4d8" }.into(),
    line: if dark_mode { "#a1a1aa" } else { "#52525b" }.into(),
    error: "#ef4444".into(),
    warning: "#f59e0b".into(),
    success: "#22c55e".into(),
  }
}

/// Parse the JSON payload returned by `mica-md-wasm`'s `render_math(source, display, color)`.
pub fn decode_math_svg_json(raw: &str) -> Option<MathSvg> {
  serde_json::from_str::<MathSvg>(raw).ok()
}

/// Render a Mermaid diagram into SVG markup for the given theme.
/// On desktop this is synchronous and cached; on Web (`wasm32`) a cache miss
/// schedules lazy loading of `mica_preview/mermaid.wasm` and bumps [`preview_rev`].
pub fn render_mermaid_cached(src: &str, dark_mode: bool) -> Option<String> {
  let clean = if src.trim().is_empty() {
    "graph LR\n  A[Mica] --> B[Pure Rust]"
  } else {
    src.trim()
  };
  let key = (dark_mode, clean.to_owned());
  if let Some(cached) = MERMAID_CACHE.with(|c| c.borrow().get(&key).cloned()) {
    return cached;
  }

  #[cfg(not(target_arch = "wasm32"))]
  {
    let host_theme = mermaid_host_theme(dark_mode);
    let rendered = mica_md::render_mermaid_svg(clean, Some(&host_theme));
    insert_mermaid_cache(key, rendered.clone());
    rendered
  }

  #[cfg(target_arch = "wasm32")]
  {
    schedule_web_mermaid(key);
    None
  }
}

/// Render a LaTeX math expression into [`MathSvg`].
/// On desktop this is synchronous and cached; on Web (`wasm32`) a cache miss
/// schedules lazy loading of `mica_preview/math.wasm` and bumps [`preview_rev`].
pub fn render_math_cached(latex: &str, display: bool, color: &str) -> Option<MathSvg> {
  let clean = latex.trim();
  if clean.is_empty() {
    return None;
  }
  let key = (display, color.to_owned(), clean.to_owned());
  if let Some(cached) = MATH_CACHE.with(|c| c.borrow().get(&key).cloned()) {
    return cached;
  }

  #[cfg(not(target_arch = "wasm32"))]
  {
    let rendered = mica_md::render::render_math_svg(clean, display, Some(color));
    insert_math_cache(key, rendered.clone());
    rendered
  }

  #[cfg(target_arch = "wasm32")]
  {
    schedule_web_math(key);
    None
  }
}

fn insert_mermaid_cache(key: (bool, String), val: Option<String>) {
  MERMAID_CACHE.with(|c| {
    let mut map = c.borrow_mut();
    if map.len() >= MAX_CACHE_ENTRIES {
      map.clear();
    }
    map.insert(key, val);
  });
}

fn insert_math_cache(key: (bool, String, String), val: Option<MathSvg>) {
  MATH_CACHE.with(|c| {
    let mut map = c.borrow_mut();
    if map.len() >= MAX_CACHE_ENTRIES {
      map.clear();
    }
    map.insert(key, val);
  });
}

#[cfg(target_arch = "wasm32")]
fn schedule_web_mermaid(key: (bool, String)) {
  let should_spawn = MERMAID_INFLIGHT.with(|set| set.borrow_mut().insert(key.clone()));
  if !should_spawn {
    return;
  }
  let (dark_mode, src) = key.clone();
  let theme_json = serde_json::to_string(&mermaid_host_theme(dark_mode)).unwrap_or_default();
  spawn_on_preview_runtime(async move {
    let rendered = call_js_preview_mermaid(&src, &theme_json).await;
    MERMAID_INFLIGHT.with(|set| {
      set.borrow_mut().remove(&key);
    });
    insert_mermaid_cache(key, rendered);
    bump_preview_rev();
  });
}

#[cfg(target_arch = "wasm32")]
fn schedule_web_math(key: (bool, String, String)) {
  let should_spawn = MATH_INFLIGHT.with(|set| set.borrow_mut().insert(key.clone()));
  if !should_spawn {
    return;
  }
  let (display, color, latex) = key.clone();
  spawn_on_preview_runtime(async move {
    let rendered = call_js_preview_math(&latex, display, &color)
      .await
      .and_then(|json| decode_math_svg_json(&json));
    MATH_INFLIGHT.with(|set| {
      set.borrow_mut().remove(&key);
    });
    insert_math_cache(key, rendered);
    bump_preview_rev();
  });
}

#[cfg(target_arch = "wasm32")]
async fn call_js_preview_mermaid(source: &str, theme_json: &str) -> Option<String> {
  use wasm_bindgen::JsCast;
  use wasm_bindgen::JsValue;

  let win = web_sys::window()?;
  let func_val = js_sys::Reflect::get(&win, &JsValue::from_str("__micaPreviewRenderMermaid")).ok()?;
  let func: js_sys::Function = func_val.dyn_into().ok()?;
  let promise_val = func
    .call2(&win, &JsValue::from_str(source), &JsValue::from_str(theme_json))
    .ok()?;
  let promise: js_sys::Promise = promise_val.dyn_into().ok()?;
  let res = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?;
  res.as_string()
}

#[cfg(target_arch = "wasm32")]
async fn call_js_preview_math(source: &str, display: bool, color: &str) -> Option<String> {
  use wasm_bindgen::JsCast;
  use wasm_bindgen::JsValue;

  let win = web_sys::window()?;
  let func_val = js_sys::Reflect::get(&win, &JsValue::from_str("__micaPreviewRenderMath")).ok()?;
  let func: js_sys::Function = func_val.dyn_into().ok()?;
  let promise_val = func
    .call3(
      &win,
      &JsValue::from_str(source),
      &JsValue::from_bool(display),
      &JsValue::from_str(color),
    )
    .ok()?;
  let promise: js_sys::Promise = promise_val.dyn_into().ok()?;
  let res = wasm_bindgen_futures::JsFuture::from(promise).await.ok()?;
  res.as_string()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn decode_math_svg_json_parses_preview_wasm_wire_format() {
    let raw = r#"{"svg":"<svg viewBox=\"0 0 10 5\"></svg>","width":2.5,"height":1.25,"baseline":0.85}"#;
    let parsed = decode_math_svg_json(raw).expect("valid math wire payload");
    assert!(parsed.svg.contains("viewBox"));
    assert!((parsed.width - 2.5).abs() < 1e-6);
    assert!((parsed.height - 1.25).abs() < 1e-6);
    assert!((parsed.baseline - 0.85).abs() < 1e-6);
    assert!(decode_math_svg_json("not json").is_none());
  }

  #[test]
  fn host_theme_distinguishes_light_and_dark_palettes() {
    let light = mermaid_host_theme(false);
    let dark = mermaid_host_theme(true);
    assert!(!light.dark && dark.dark);
    assert_ne!(light.canvas, dark.canvas);
    assert_ne!(light.text, dark.text);
  }
}
