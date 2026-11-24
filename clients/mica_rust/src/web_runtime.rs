//! Browser platform helpers for the pure-Rust WebAssembly (`wasm32`) target.
//!
//! Isolates `web_sys` calls behind `#[cfg(target_arch = "wasm32")]` so the rest
//! of `mica_rust` compiles cleanly on native desktop targets with zero browser
//! dependencies while Web runs as a cloud-only, same-origin client backed by
//! `window.localStorage` and `XMLHttpRequest`.

/// Returns `true` when compiled for WebAssembly (`wasm32`), where Mica runs as
/// a pure cloud client with no on-device local workspace.
pub const fn is_web() -> bool {
  cfg!(target_arch = "wasm32")
}

/// Returns the browser page's canonical origin (e.g. `https://mica.example.com`
/// or `http://127.0.0.1:8090`) on `wasm32`, or `None` on native desktop.
pub fn web_origin() -> Option<String> {
  #[cfg(target_arch = "wasm32")]
  {
    let origin = web_sys::window()?.location().origin().ok()?;
    let trimmed = origin.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("null") {
      None
    } else {
      Some(trimmed.to_string())
    }
  }
  #[cfg(not(target_arch = "wasm32"))]
  {
    None
  }
}

/// Read a key from `window.localStorage` on `wasm32` (always `None` on desktop).
pub fn local_storage_get(key: &str) -> Option<String> {
  #[cfg(target_arch = "wasm32")]
  {
    let storage = web_sys::window()?.local_storage().ok()??;
    storage.get_item(key).ok()?
  }
  #[cfg(not(target_arch = "wasm32"))]
  {
    let _ = key;
    None
  }
}

/// Write a key to `window.localStorage` on `wasm32` (no-op on desktop).
pub fn local_storage_set(key: &str, value: &str) -> Result<(), String> {
  #[cfg(target_arch = "wasm32")]
  {
    let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) else {
      return Ok(());
    };
    storage
      .set_item(key, value)
      .map_err(|e| format!("failed to write localStorage[{key}]: {e:?}"))
  }
  #[cfg(not(target_arch = "wasm32"))]
  {
    let _ = (key, value);
    Ok(())
  }
}

/// Execute a same-origin / CORS HTTP request via `XMLHttpRequest` on `wasm32`.
///
/// Uses synchronous XHR (`async = false`) so the existing synchronous
/// `CloudConnector` API works identically across Desktop (`ureq`) and Web
/// without duplicating endpoint codecs. Deliberately avoids setting `timeout`
/// or `responseType` on synchronous XHR per the browser XHR specification.
#[cfg(target_arch = "wasm32")]
pub fn xhr_request(
  method: &str,
  url: &str,
  bearer_token: Option<&str>,
  json_body: Option<&str>,
) -> Result<String, String> {
  let xhr = web_sys::XmlHttpRequest::new()
    .map_err(|e| format!("failed to construct XMLHttpRequest: {e:?}"))?;
  xhr
    .open_with_async(method, url, false)
    .map_err(|e| format!("{method} {url} open failed: {e:?}"))?;
  let _ = xhr.set_request_header("Accept", "application/json");
  if let Some(tok) = bearer_token.map(str::trim).filter(|t| !t.is_empty()) {
    let _ = xhr.set_request_header("Authorization", &format!("Bearer {tok}"));
  }
  if json_body.is_some() {
    let _ = xhr.set_request_header("Content-Type", "application/json");
  }
  xhr
    .send_with_opt_str(json_body)
    .map_err(|e| format!("{method} {url} network error: {e:?}"))?;

  let status = xhr.status().unwrap_or(0);
  let text = xhr.response_text().ok().flatten().unwrap_or_default();
  if (200..300).contains(&status) {
    return Ok(text);
  }

  let detail = extract_server_error_message(&text);
  if detail.is_empty() {
    Err(format!("HTTP {status}"))
  } else {
    Err(format!("HTTP {status}: {detail}"))
  }
}

/// Trigger a real browser file download (`Blob` + `<a download>` + `URL.revokeObjectURL`)
/// on `wasm32`. Returns an error on native desktop (which uses OS Save-As dialogs).
pub fn web_download_bytes(filename: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
  #[cfg(target_arch = "wasm32")]
  {
    use wasm_bindgen::JsCast;
    use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

    let window = web_sys::window().ok_or_else(|| "window unavailable".to_string())?;
    let document = window.document().ok_or_else(|| "document unavailable".to_string())?;
    let body = document.body().ok_or_else(|| "document.body unavailable".to_string())?;

    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let opts = BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &opts)
      .map_err(|e| format!("failed to create Blob: {e:?}"))?;
    let url = Url::create_object_url_with_blob(&blob)
      .map_err(|e| format!("failed to create object URL: {e:?}"))?;

    let el = document
      .create_element("a")
      .map_err(|e| format!("failed to create anchor element: {e:?}"))?;
    let anchor: HtmlAnchorElement = el
      .dyn_into()
      .map_err(|_| "created element was not HtmlAnchorElement".to_string())?;
    anchor.set_href(&url);
    anchor.set_download(filename);
    let _ = anchor.set_attribute("style", "display:none");
    let _ = body.append_child(&anchor);
    anchor.click();
    let _ = body.remove_child(&anchor);
    let _ = Url::revoke_object_url(&url);
    Ok(())
  }
  #[cfg(not(target_arch = "wasm32"))]
  {
    let _ = (filename, mime, bytes);
    Err("browser file download is only available on wasm32".to_string())
  }
}

/// Open a hidden `<input type="file">` picker in the browser and read all
/// selected files into `(filename, bytes)` pairs via `FileReader`.
/// Returns `Ok(Vec::new())` if the user cancels.
#[cfg(target_arch = "wasm32")]
pub async fn web_pick_files(accept: &str, multiple: bool) -> Result<Vec<(String, Vec<u8>)>, String> {
  use std::cell::RefCell;
  use std::future::Future;
  use std::pin::Pin;
  use std::rc::Rc;
  use std::task::{Context, Poll, Waker};

  use wasm_bindgen::JsCast;
  use wasm_bindgen::closure::Closure;
  use web_sys::{Event, File, FileReader, HtmlInputElement};

  struct PickState {
    result: Option<Result<Vec<(String, Vec<u8>)>, String>>,
    waker: Option<Waker>,
    input: Option<HtmlInputElement>,
    _on_change: Option<Closure<dyn FnMut(Event)>>,
    _on_cancel: Option<Closure<dyn FnMut(Event)>>,
    _readers: Vec<(FileReader, Closure<dyn FnMut(Event)>, Closure<dyn FnMut(Event)>)>,
  }

  impl PickState {
    fn finish(&mut self, res: Result<Vec<(String, Vec<u8>)>, String>) {
      if self.result.is_some() {
        return;
      }
      if let Some(input) = self.input.take() {
        input.set_onchange(None);
        input.set_oncancel(None);
        input.remove();
      }
      self._on_change = None;
      self._on_cancel = None;
      self._readers.clear();
      self.result = Some(res);
      if let Some(w) = self.waker.take() {
        w.wake();
      }
    }
  }

  struct PickFuture {
    state: Rc<RefCell<PickState>>,
  }

  impl Future for PickFuture {
    type Output = Result<Vec<(String, Vec<u8>)>, String>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
      let mut st = self.state.borrow_mut();
      if let Some(res) = st.result.take() {
        Poll::Ready(res)
      } else {
        st.waker = Some(cx.waker().clone());
        Poll::Pending
      }
    }
  }

  let window = web_sys::window().ok_or_else(|| "window unavailable".to_string())?;
  let document = window.document().ok_or_else(|| "document unavailable".to_string())?;
  let body = document.body().ok_or_else(|| "document.body unavailable".to_string())?;

  let el = document
    .create_element("input")
    .map_err(|e| format!("failed to create input element: {e:?}"))?;
  let input: HtmlInputElement = el
    .dyn_into()
    .map_err(|_| "created element was not HtmlInputElement".to_string())?;
  input.set_type("file");
  if !accept.trim().is_empty() {
    input.set_accept(accept);
  }
  input.set_multiple(multiple);
  let _ = input.set_attribute("style", "display:none");
  let _ = body.append_child(&input);

  let state = Rc::new(RefCell::new(PickState {
    result: None,
    waker: None,
    input: Some(input.clone()),
    _on_change: None,
    _on_cancel: None,
    _readers: Vec::new(),
  }));

  let st_cancel = Rc::clone(&state);
  let on_cancel = Closure::<dyn FnMut(Event)>::new(move |_ev: Event| {
    st_cancel.borrow_mut().finish(Ok(Vec::new()));
  });
  input.set_oncancel(Some(on_cancel.as_ref().unchecked_ref()));

  let st_change = Rc::clone(&state);
  let input_ref = input.clone();
  let on_change = Closure::<dyn FnMut(Event)>::new(move |_ev: Event| {
    let Some(file_list) = input_ref.files() else {
      st_change.borrow_mut().finish(Ok(Vec::new()));
      return;
    };
    let count = file_list.length() as usize;
    if count == 0 {
      st_change.borrow_mut().finish(Ok(Vec::new()));
      return;
    }

    let collected: Rc<RefCell<Vec<Option<(String, Vec<u8>)>>>> =
      Rc::new(RefCell::new(vec![None; count]));
    let remaining = Rc::new(RefCell::new(count));
    let mut readers = Vec::with_capacity(count);

    for idx in 0..count {
      let Some(file): Option<File> = file_list.get(idx as u32) else {
        let mut rem = remaining.borrow_mut();
        *rem = rem.saturating_sub(1);
        continue;
      };
      let name = file.name();
      let Ok(reader) = FileReader::new() else {
        st_change
          .borrow_mut()
          .finish(Err("failed to create FileReader".to_string()));
        return;
      };

      let r_ok = reader.clone();
      let col_ok = Rc::clone(&collected);
      let rem_ok = Rc::clone(&remaining);
      let st_ok = Rc::clone(&st_change);
      let name_ok = name.clone();
      let onload = Closure::<dyn FnMut(Event)>::new(move |_e: Event| {
        match r_ok.result() {
          Ok(buf) => {
            let bytes = js_sys::Uint8Array::new(&buf).to_vec();
            col_ok.borrow_mut()[idx] = Some((name_ok.clone(), bytes));
            let done = {
              let mut rem = rem_ok.borrow_mut();
              *rem = rem.saturating_sub(1);
              *rem == 0
            };
            if done {
              let out: Vec<(String, Vec<u8>)> = col_ok
                .borrow_mut()
                .drain(..)
                .flatten()
                .collect();
              st_ok.borrow_mut().finish(Ok(out));
            }
          }
          Err(err) => {
            st_ok
              .borrow_mut()
              .finish(Err(format!("failed to read selected file {name_ok}: {err:?}")));
          }
        }
      });

      let st_err = Rc::clone(&st_change);
      let name_err = name.clone();
      let onerror = Closure::<dyn FnMut(Event)>::new(move |_e: Event| {
        st_err
          .borrow_mut()
          .finish(Err(format!("failed to read selected file {name_err}")));
      });

      reader.set_onload(Some(onload.as_ref().unchecked_ref()));
      reader.set_onerror(Some(onerror.as_ref().unchecked_ref()));
      if let Err(err) = reader.read_as_array_buffer(&file) {
        st_change
          .borrow_mut()
          .finish(Err(format!("read_as_array_buffer failed for {name}: {err:?}")));
        return;
      }
      readers.push((reader, onload, onerror));
    }

    st_change.borrow_mut()._readers = readers;
  });
  input.set_onchange(Some(on_change.as_ref().unchecked_ref()));

  {
    let mut st = state.borrow_mut();
    st._on_change = Some(on_change);
    st._on_cancel = Some(on_cancel);
  }
  input.click();

  PickFuture { state }.await
}

#[cfg(any(target_arch = "wasm32", test))]
fn extract_server_error_message(raw_body: &str) -> String {
  let trimmed = raw_body.trim();
  if trimmed.is_empty() {
    return String::new();
  }
  if let Ok(val) = serde_json::from_str::<serde_json::Value>(trimmed) {
    for key in ["error", "message", "detail"] {
      if let Some(msg) = val.get(key).and_then(serde_json::Value::as_str) {
        let m = msg.trim();
        if !m.is_empty() {
          return m.to_string();
        }
      }
    }
  }
  trimmed.chars().take(240).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn extract_server_error_reads_json_error_or_plain_text() {
    assert_eq!(
      extract_server_error_message(r#"{"error":"Invalid email or password"}"#),
      "Invalid email or password"
    );
    assert_eq!(
      extract_server_error_message(r#"{"message":"Unauthorized"}"#),
      "Unauthorized"
    );
    assert_eq!(extract_server_error_message("bad gateway"), "bad gateway");
    assert_eq!(extract_server_error_message("   "), "");
  }

  #[test]
  fn desktop_reports_non_web_runtime() {
    assert!(!is_web());
    assert!(web_origin().is_none());
  }
}
