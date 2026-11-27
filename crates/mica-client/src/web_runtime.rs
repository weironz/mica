//! Browser platform helpers for the pure-Rust WebAssembly (`wasm32`) target.
//!
//! Isolates `web_sys` calls behind `#[cfg(target_arch = "wasm32")]` so the rest
//! of `mica_rust` compiles cleanly on native desktop targets with zero browser
//! dependencies while Web runs as a cloud-only, same-origin client backed by
//! async `fetch` (with `AbortController` timeouts), `HttpOnly` cookies,
//! `window.localStorage` (non-secret UI prefs only), and IndexedDB.

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

/// Consumes an ephemeral in-memory pre-authentication JSON payload placed on
/// `window.__micaPreAuth` by `index.html`'s fast boot shell (either from an
/// early cookie refresh or from the instant HTML login form), deleting the
/// property immediately so secrets never linger on `window` or `localStorage`.
pub fn take_web_preauth_json() -> Option<String> {
  #[cfg(target_arch = "wasm32")]
  {
    let win = web_sys::window()?;
    let key = wasm_bindgen::JsValue::from_str("__micaPreAuth");
    let val = js_sys::Reflect::get(&win, &key).ok()?;
    let _ = js_sys::Reflect::delete_property(&win, &key);
    let raw = val.as_string()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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

const WEB_LAST_REFRESH_KEY: &str = "mica_auth_refreshed_at";

/// Non-secret epoch-millisecond timestamp recorded whenever any browser tab on
/// this origin completes `POST /api/auth/refresh`, letting sibling tabs skip
/// redundant rotations after acquiring the cross-tab Web Lock.
pub fn web_last_refresh_ms() -> u64 {
  local_storage_get(WEB_LAST_REFRESH_KEY)
    .and_then(|raw| raw.trim().parse::<u64>().ok())
    .unwrap_or(0)
}

/// Record the current epoch-millisecond timestamp after a successful Web
/// `POST /api/auth/refresh`. Never writes any token or secret.
pub fn mark_web_refreshed_now() {
  #[cfg(target_arch = "wasm32")]
  {
    let now = js_sys::Date::now().max(0.0) as u64;
    let _ = local_storage_set(WEB_LAST_REFRESH_KEY, &now.to_string());
  }
}

/// Serialize an async operation across all browser tabs on the same origin via
/// the Web Locks API (`navigator.locks.request`), falling back to running `f`
/// directly on desktop or in browsers without `navigator.locks`.
pub async fn with_cross_tab_lock<T, F, Fut>(lock_name: &str, f: F) -> Result<T, String>
where
  T: 'static,
  F: FnOnce() -> Fut + 'static,
  Fut: std::future::Future<Output = Result<T, String>> + 'static,
{
  #[cfg(not(target_arch = "wasm32"))]
  {
    let _ = lock_name;
    f().await
  }

  #[cfg(target_arch = "wasm32")]
  {
    use std::cell::RefCell;
    use std::rc::Rc;

    use js_sys::{Function, Promise, Reflect};
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::{JsFuture, future_to_promise};

    let locks_req = web_sys::window().and_then(|win| {
      let nav = Reflect::get(&win, &JsValue::from_str("navigator")).ok()?;
      let locks = Reflect::get(&nav, &JsValue::from_str("locks")).ok()?;
      if locks.is_undefined() || locks.is_null() {
        return None;
      }
      let req = Reflect::get(&locks, &JsValue::from_str("request"))
        .ok()?
        .dyn_into::<Function>()
        .ok()?;
      Some((locks, req))
    });

    let Some((locks, req_fn)) = locks_req else {
      return f().await;
    };

    let result_slot: Rc<RefCell<Option<Result<T, String>>>> = Rc::new(RefCell::new(None));
    let slot_cb = Rc::clone(&result_slot);
    let f_cell = Rc::new(RefCell::new(Some(f)));
    let f_cb = Rc::clone(&f_cell);

    let cb = Closure::once(move |_lock: JsValue| -> Promise {
      let op = f_cb.borrow_mut().take();
      future_to_promise(async move {
        if let Some(run) = op {
          let res = run().await;
          *slot_cb.borrow_mut() = Some(res);
        }
        Ok(JsValue::UNDEFINED)
      })
    });

    let promise_val = match req_fn.call2(
      &locks,
      &JsValue::from_str(lock_name),
      cb.as_ref().unchecked_ref(),
    ) {
      Ok(val) => val,
      Err(_) => {
        let run = f_cell.borrow_mut().take();
        if let Some(run) = run {
          return run().await;
        }
        return Err("cross-tab lock failed before callback".to_string());
      }
    };

    let Ok(promise) = promise_val.dyn_into::<Promise>() else {
      let run = f_cell.borrow_mut().take();
      if let Some(run) = run {
        return run().await;
      }
      return Err("navigator.locks.request did not return a Promise".to_string());
    };

    let _ = JsFuture::from(promise).await;
    drop(cb);
    result_slot
      .borrow_mut()
      .take()
      .unwrap_or_else(|| Err("cross-tab lock exited without producing a result".to_string()))
  }
}

/// Execute an asynchronous same-origin HTTP request via the browser Fetch API
/// (`window.fetch`) with an `AbortController` timeout and `credentials: "same-origin"`
/// so `HttpOnly` session cookies are sent and updated without ever blocking the
/// UI thread.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_request(
  method: &str,
  url: &str,
  bearer_token: Option<&str>,
  json_body: Option<&str>,
  timeout_secs: u64,
) -> Result<String, String> {
  use wasm_bindgen::closure::Closure;
  use wasm_bindgen::{JsCast, JsValue};
  use wasm_bindgen_futures::JsFuture;
  use web_sys::{AbortController, Headers, Request, RequestCredentials, RequestInit, Response};

  struct TimeoutGuard {
    window: web_sys::Window,
    timer_id: i32,
    _cb: Closure<dyn FnMut()>,
  }

  impl Drop for TimeoutGuard {
    fn drop(&mut self) {
      self.window.clear_timeout_with_handle(self.timer_id);
    }
  }

  let window = web_sys::window().ok_or_else(|| "window unavailable".to_string())?;
  let controller =
    AbortController::new().map_err(|e| format!("failed to construct AbortController: {e:?}"))?;
  let ctrl_abort = controller.clone();
  let timed_out = std::rc::Rc::new(std::cell::Cell::new(false));
  let timed_out_cb = std::rc::Rc::clone(&timed_out);
  let abort_cb = Closure::<dyn FnMut()>::new(move || {
    timed_out_cb.set(true);
    ctrl_abort.abort();
  });
  let timeout_ms = timeout_secs.clamp(1, 600).saturating_mul(1000) as i32;
  let timer_id = window
    .set_timeout_with_callback_and_timeout_and_arguments_0(
      abort_cb.as_ref().unchecked_ref(),
      timeout_ms,
    )
    .map_err(|e| format!("failed to schedule fetch timeout: {e:?}"))?;
  let _guard = TimeoutGuard {
    window: window.clone(),
    timer_id,
    _cb: abort_cb,
  };

  let headers = Headers::new().map_err(|e| format!("failed to construct Headers: {e:?}"))?;
  let _ = headers.set("Accept", "application/json");
  if let Some(tok) = bearer_token.map(str::trim).filter(|t| !t.is_empty()) {
    let _ = headers.set("Authorization", &format!("Bearer {tok}"));
  }
  if json_body.is_some() {
    let _ = headers.set("Content-Type", "application/json");
  }

  let init = RequestInit::new();
  init.set_method(method);
  init.set_headers(&headers);
  init.set_credentials(RequestCredentials::SameOrigin);
  init.set_signal(Some(&controller.signal()));
  if let Some(body) = json_body {
    init.set_body(&JsValue::from_str(body));
  }

  let request = Request::new_with_str_and_init(url, &init)
    .map_err(|e| format!("{method} {url} request init failed: {e:?}"))?;

  let resp_val = JsFuture::from(window.fetch_with_request(&request))
    .await
    .map_err(|e| {
      if timed_out.get() {
        format!("{method} {url} timed out after {timeout_secs}s")
      } else {
        format!("{method} {url} network error: {e:?}")
      }
    })?;
  let resp: Response = resp_val
    .dyn_into()
    .map_err(|_| format!("{method} {url} returned non-Response object"))?;

  let status = resp.status();
  let text_promise = resp
    .text()
    .map_err(|e| format!("{method} {url} response.text() failed: {e:?}"))?;
  let text = JsFuture::from(text_promise)
    .await
    .map_err(|e| {
      if timed_out.get() {
        format!("{method} {url} body timed out after {timeout_secs}s")
      } else {
        format!("{method} {url} body read failed: {e:?}")
      }
    })?
    .as_string()
    .unwrap_or_default();

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

/// Fetch binary bytes from `url` via the browser Fetch API (`GET`), returning
/// `(bytes, content_type)`. Uses `same-origin` credentials for same-origin URLs
/// and `omit` for cross-origin URLs so external images/S3 redirects don't fail
/// strict CORS credential rules.
#[cfg(target_arch = "wasm32")]
pub async fn fetch_bytes(url: &str, timeout_secs: u64) -> Result<(Vec<u8>, String), String> {
  use wasm_bindgen::JsCast;
  use wasm_bindgen::closure::Closure;
  use wasm_bindgen_futures::JsFuture;
  use web_sys::{AbortController, Request, RequestCredentials, RequestInit, Response};

  struct TimeoutGuard {
    window: web_sys::Window,
    timer_id: i32,
    _cb: Closure<dyn FnMut()>,
  }
  impl Drop for TimeoutGuard {
    fn drop(&mut self) {
      self.window.clear_timeout_with_handle(self.timer_id);
    }
  }

  let window = web_sys::window().ok_or_else(|| "window unavailable".to_string())?;
  let controller =
    AbortController::new().map_err(|e| format!("failed to construct AbortController: {e:?}"))?;
  let ctrl_abort = controller.clone();
  let timed_out = std::rc::Rc::new(std::cell::Cell::new(false));
  let timed_out_cb = std::rc::Rc::clone(&timed_out);
  let abort_cb = Closure::<dyn FnMut()>::new(move || {
    timed_out_cb.set(true);
    ctrl_abort.abort();
  });
  let timeout_ms = timeout_secs.clamp(1, 600).saturating_mul(1000) as i32;
  let timer_id = window
    .set_timeout_with_callback_and_timeout_and_arguments_0(
      abort_cb.as_ref().unchecked_ref(),
      timeout_ms,
    )
    .map_err(|e| format!("failed to schedule fetch timeout: {e:?}"))?;
  let _guard = TimeoutGuard {
    window: window.clone(),
    timer_id,
    _cb: abort_cb,
  };

  let init = RequestInit::new();
  init.set_method("GET");
  let same_origin = web_origin().is_some_and(|orig| url.starts_with(&orig));
  init.set_credentials(if same_origin {
    RequestCredentials::SameOrigin
  } else {
    RequestCredentials::Omit
  });
  init.set_signal(Some(&controller.signal()));

  let request = Request::new_with_str_and_init(url, &init)
    .map_err(|e| format!("GET {url} request init failed: {e:?}"))?;
  let resp_val = JsFuture::from(window.fetch_with_request(&request))
    .await
    .map_err(|e| {
      if timed_out.get() {
        format!("GET {url} timed out after {timeout_secs}s")
      } else {
        format!("GET {url} network error: {e:?}")
      }
    })?;
  let resp: Response = resp_val
    .dyn_into()
    .map_err(|_| format!("GET {url} returned non-Response object"))?;
  let status = resp.status();
  if !(200..300).contains(&status) {
    return Err(format!("HTTP {status}"));
  }
  let content_type = resp
    .headers()
    .get("content-type")
    .ok()
    .flatten()
    .unwrap_or_default();
  let buf_promise = resp
    .array_buffer()
    .map_err(|e| format!("GET {url} response.arrayBuffer() failed: {e:?}"))?;
  let buf_val = JsFuture::from(buf_promise)
    .await
    .map_err(|e| format!("GET {url} body read failed: {e:?}"))?;
  let bytes = js_sys::Uint8Array::new(&buf_val).to_vec();
  Ok((bytes, content_type))
}

/// PUT raw bytes to a presigned object-storage URL with explicit SigV4 headers
/// and `credentials: "omit"`, returning the HTTP status code.
#[cfg(target_arch = "wasm32")]
pub async fn put_presigned_bytes(
  url: &str,
  headers_kv: &[(&str, &str)],
  body: &[u8],
  timeout_secs: u64,
) -> Result<u16, String> {
  use wasm_bindgen::JsCast;
  use wasm_bindgen::closure::Closure;
  use wasm_bindgen_futures::JsFuture;
  use web_sys::{AbortController, Headers, Request, RequestCredentials, RequestInit, Response};

  struct TimeoutGuard {
    window: web_sys::Window,
    timer_id: i32,
    _cb: Closure<dyn FnMut()>,
  }
  impl Drop for TimeoutGuard {
    fn drop(&mut self) {
      self.window.clear_timeout_with_handle(self.timer_id);
    }
  }

  let window = web_sys::window().ok_or_else(|| "window unavailable".to_string())?;
  let controller =
    AbortController::new().map_err(|e| format!("failed to construct AbortController: {e:?}"))?;
  let ctrl_abort = controller.clone();
  let timed_out = std::rc::Rc::new(std::cell::Cell::new(false));
  let timed_out_cb = std::rc::Rc::clone(&timed_out);
  let abort_cb = Closure::<dyn FnMut()>::new(move || {
    timed_out_cb.set(true);
    ctrl_abort.abort();
  });
  let timeout_ms = timeout_secs.clamp(1, 600).saturating_mul(1000) as i32;
  let timer_id = window
    .set_timeout_with_callback_and_timeout_and_arguments_0(
      abort_cb.as_ref().unchecked_ref(),
      timeout_ms,
    )
    .map_err(|e| format!("failed to schedule PUT timeout: {e:?}"))?;
  let _guard = TimeoutGuard {
    window: window.clone(),
    timer_id,
    _cb: abort_cb,
  };

  let headers = Headers::new().map_err(|e| format!("failed to construct Headers: {e:?}"))?;
  for &(k, v) in headers_kv {
    let _ = headers.set(k, v);
  }
  let init = RequestInit::new();
  init.set_method("PUT");
  init.set_headers(&headers);
  init.set_credentials(RequestCredentials::Omit);
  init.set_signal(Some(&controller.signal()));
  let arr = js_sys::Uint8Array::from(body);
  init.set_body(&arr);

  let request = Request::new_with_str_and_init(url, &init)
    .map_err(|e| format!("PUT {url} request init failed: {e:?}"))?;
  let resp_val = JsFuture::from(window.fetch_with_request(&request))
    .await
    .map_err(|e| {
      if timed_out.get() {
        format!("PUT {url} timed out after {timeout_secs}s")
      } else {
        format!("PUT {url} network error: {e:?}")
      }
    })?;
  let resp: Response = resp_val
    .dyn_into()
    .map_err(|_| format!("PUT {url} returned non-Response object"))?;
  Ok(resp.status())
}

/// Trigger a real browser file download (`Blob` + `<a download>` + `URL.revokeObjectURL`)
/// on `wasm32`. Returns an error on native desktop (which uses OS Save-As dialogs).
pub fn web_download_bytes(filename: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
  #[cfg(target_arch = "wasm32")]
  {
    use wasm_bindgen::JsCast;
    use web_sys::{Blob, BlobPropertyBag, HtmlAnchorElement, Url};

    let window = web_sys::window().ok_or_else(|| "window unavailable".to_string())?;
    let document = window
      .document()
      .ok_or_else(|| "document unavailable".to_string())?;
    let body = document
      .body()
      .ok_or_else(|| "document.body unavailable".to_string())?;

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
pub async fn web_pick_files(
  accept: &str,
  multiple: bool,
) -> Result<Vec<(String, Vec<u8>)>, String> {
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
    _readers: Vec<(
      FileReader,
      Closure<dyn FnMut(Event)>,
      Closure<dyn FnMut(Event)>,
    )>,
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
  let document = window
    .document()
    .ok_or_else(|| "document unavailable".to_string())?;
  let body = document
    .body()
    .ok_or_else(|| "document.body unavailable".to_string())?;

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
      let onload = Closure::<dyn FnMut(Event)>::new(move |_e: Event| match r_ok.result() {
        Ok(buf) => {
          let bytes = js_sys::Uint8Array::new(&buf).to_vec();
          col_ok.borrow_mut()[idx] = Some((name_ok.clone(), bytes));
          let done = {
            let mut rem = rem_ok.borrow_mut();
            *rem = rem.saturating_sub(1);
            *rem == 0
          };
          if done {
            let out: Vec<(String, Vec<u8>)> = col_ok.borrow_mut().drain(..).flatten().collect();
            st_ok.borrow_mut().finish(Ok(out));
          }
        }
        Err(err) => {
          st_ok.borrow_mut().finish(Err(format!(
            "failed to read selected file {name_ok}: {err:?}"
          )));
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
        st_change.borrow_mut().finish(Err(format!(
          "read_as_array_buffer failed for {name}: {err:?}"
        )));
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
