//! Full-featured image block lifecycle & UI for Mica's pure-Rust L3 shell.
//!
//! Restores 1:1 parity with `v0.14.11` across Desktop and WebAssembly:
//! - Resolves `data.file_id` (Mica cloud `/api/workspaces/{ws}/files/{id}/blob/{name}`
//!   and Desktop on-device CAS mirror for instant offline reads) as well as external
//!   `data.url` / `data.src` and legacy `file://` / `data:` URIs.
//! - Uploads picked / pasted images to cloud storage (`presign → PUT → complete`)
//!   or Desktop local CAS, storing `file_id + name` in CRDT blocks so images sync
//!   cleanly across Desktop, Web, and collaborators.
//! - Single-click block selection (blue border, `Ctrl+C` / `Ctrl+X` / `Delete`),
//!   double-click fullscreen zoom lightbox, true natural aspect ratio via `on_load`,
//!   hover toolbar (`放大 / 复制 / 左 / 中 / 右 / 图注 / 删除`), right-edge drag
//!   resize handle (`data.width`), external-link badge + server/client re-hosting,
//!   and complete right-click context menu (copy bitmap, cut, copy link, open in
//!   browser, replace, paste-replace, download/save-as, delete).

use std::{
  cell::RefCell,
  collections::{BTreeMap, HashMap},
  rc::Rc,
  time::Duration,
};

#[cfg(not(target_arch = "wasm32"))]
use mica_client::sha256_hex;
use mica_client::{build_file_blob_url, download_url_bytes, parse_mica_blob_link};
use mica_editor::{DocSelection, EditorBlock};
use nsh_base::{button, input};
use nsh_core::{
  ContextMenuEvent, Cursor, MouseEvent, PointerEvents, SubmitBehavior,
  element::{ImageSrc, ResponderEvent, TouchClaim, overlay, set_clipboard},
  geometry::{DevicePixels, Size, px},
  prelude::*,
};
use serde_json::Value;
use uuid::Uuid;

use crate::{
  block_views::{BlockRenderEnv, LightboxContent},
  clipboard_platform::{
    copy_image_bytes_to_clipboard, decode_data_uri, mime_for_image, probe_local_image_dimensions,
  },
  cloud_sync::{canonicalize_cloud_url, with_auto_refresh},
  editor_bridge::MicaDocumentBridge,
  mica_theme::palette,
  net::CloudConnector,
};

const IMAGE_PICK_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg"];

/// Shared reactive context for resolving, uploading, caching, and acting on images.
#[derive(Clone)]
pub struct ImageContext {
  /// App-root reactive owner so async image tasks (copy, cut, download, upload,
  /// replace, toast dismissal) survive closing transient context menus / hover bars.
  pub owner: Owner,
  pub is_cloud_signed_in: Signal<bool>,
  pub server_url: Signal<String>,
  pub active_workspace_id: Signal<Uuid>,
  pub active_doc: Signal<Option<Uuid>>,
  pub cloud_connector: Rc<RefCell<CloudConnector>>,
  pub rehost_images: Signal<bool>,
  /// Decoded natural `(width, height)` cache keyed by display URI so recycled
  /// `virtual_list` cells render at exact aspect ratio without layout jumps.
  pub natural_sizes: Rc<RefCell<HashMap<String, (f32, f32)>>>,
  /// Active right-click context menu state (rendered in top-level `overlay()`).
  pub context_menu: Signal<Option<ImageContextMenuState>>,
  /// Active caption edit dialog state.
  pub caption_edit: Signal<Option<ImageCaptionEditState>>,
  /// Transient bottom-center feedback toast `(message, generation)`.
  pub toast_state: Signal<Option<(String, u64)>>,
}

impl ImageContext {
  pub fn new(
    is_cloud_signed_in: Signal<bool>,
    server_url: Signal<String>,
    active_workspace_id: Signal<Uuid>,
    active_doc: Signal<Option<Uuid>>,
    cloud_connector: Rc<RefCell<CloudConnector>>,
    rehost_images: Signal<bool>,
  ) -> Self {
    Self {
      owner: Owner::current(),
      is_cloud_signed_in,
      server_url,
      active_workspace_id,
      active_doc,
      cloud_connector,
      rehost_images,
      natural_sizes: Rc::new(RefCell::new(HashMap::new())),
      context_menu: signal(None),
      caption_edit: signal(None),
      toast_state: signal(None),
    }
  }

  /// Spawn an async image action under the app-root `owner` so closing a
  /// right-click context menu (`menu_sig.set(None)`) or un-hovering the image
  /// toolbar NEVER cancels the in-flight task.
  pub fn spawn<T: 'static>(&self, future: impl std::future::Future<Output = T> + 'static) {
    self.owner.run(move || {
      drop(spawn(future));
    });
  }

  /// Minimal offline/test context for headless unit tests that don't mount the full shell.
  pub fn offline_default() -> Self {
    Self::new(
      signal(false),
      signal(String::new()),
      signal(Uuid::nil()),
      signal(None),
      Rc::new(RefCell::new(CloudConnector::new(Default::default()))),
      signal(true),
    )
  }

  /// Canonical cloud server origin (`https://...`), falling back to browser origin on Web.
  pub fn effective_cloud_origin(&self) -> Option<String> {
    let raw = self.server_url.get_untracked();
    let canon = canonicalize_cloud_url(&raw);
    if !canon.is_empty() {
      return Some(canon);
    }
    mica_client::web_runtime::web_origin()
  }

  /// Show a short transient toast message inside the editor viewport.
  pub fn show_toast(&self, msg: impl Into<String>) {
    let text_msg = msg.into();
    let next_gen = self
      .toast_state
      .get_untracked()
      .map(|(_, g)| g.wrapping_add(1))
      .unwrap_or(1);
    self.toast_state.set(Some((text_msg, next_gen)));
    let toast_sig = self.toast_state;
    self.spawn(async move {
      timer(Duration::from_millis(2200)).await;
      if toast_sig.get_untracked().is_some_and(|(_, g)| g == next_gen) {
        toast_sig.set(None);
      }
    });
  }
}

/// Active right-click image context menu descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageContextMenuState {
  pub block_id: String,
  pub block_index: usize,
  pub resolved: ResolvedImage,
  pub x: f32,
  pub y: f32,
}

/// Active caption editor modal descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct ImageCaptionEditState {
  pub block_id: String,
  pub block_index: usize,
  pub initial: String,
}

/// Resolved display/share/storage metadata for a single `image` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedImage {
  /// URI passed to `image(ImageSrc::from(...))` (`file:///...`, `https://.../blob/...`,
  /// external `http(s)://...`, or `data:...`). Empty only when the block truly has no source.
  pub display_uri: String,
  /// Shareable link for "复制链接" / "在浏览器打开" / selection copy.
  pub link_url: Option<String>,
  /// `Some(file_id)` when stored in Mica cloud storage or Desktop local CAS.
  pub file_id: Option<String>,
  /// `Some(url)` when pointing to an un-rehosted external `http(s)://` URL.
  pub external_url: Option<String>,
  /// Best filename for download / save-as.
  pub file_name: String,
  /// Caption (`data.caption`, falling back to `block.text` or `data.alt`).
  pub caption: String,
  /// True when backed by Mica cloud storage (as opposed to local-only CAS or external URL).
  pub is_cloud_stored: bool,
}

impl ResolvedImage {
  pub fn is_external(&self) -> bool {
    self.file_id.is_none() && self.external_url.is_some()
  }

  pub fn external_host(&self) -> String {
    let raw = self.external_url.as_deref().unwrap_or(&self.display_uri);
    extract_url_host(raw)
  }
}

/// Extract the hostname from an `http(s)://` URL for badges and error captions.
pub fn extract_url_host(url: &str) -> String {
  let rest = url
    .trim()
    .strip_prefix("https://")
    .or_else(|| url.trim().strip_prefix("http://"))
    .unwrap_or(url.trim());
  let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
  if authority.is_empty() {
    url.trim().to_string()
  } else {
    authority.to_string()
  }
}

fn is_http_url(s: &str) -> bool {
  s.starts_with("http://") || s.starts_with("https://")
}

/// Resolve an `image` block's `data.file_id` / `data.url` / `data.src` into display
/// and shareable URIs for the current world (Cloud vs Local).
pub fn resolve_image_block(block: &EditorBlock, ctx: &ImageContext) -> ResolvedImage {
  let file_id = block
    .data
    .get("file_id")
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(str::to_owned);
  let raw_url = block
    .data
    .get("url")
    .and_then(Value::as_str)
    .or_else(|| block.data.get("src").and_then(Value::as_str))
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(str::to_owned);
  let raw_name = block
    .data
    .get("name")
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .map(str::to_owned);
  let caption = block
    .data
    .get("caption")
    .and_then(Value::as_str)
    .map(str::trim)
    .filter(|s| !s.is_empty())
    .or_else(|| {
      let t = block.text.trim();
      (!t.is_empty()).then_some(t)
    })
    .or_else(|| {
      block
        .data
        .get("alt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    })
    .unwrap_or("")
    .to_owned();

  let file_name = raw_name
    .clone()
    .or_else(|| raw_url.as_deref().and_then(infer_filename_from_url))
    .unwrap_or_else(|| "image.png".to_string());

  if let Some(fid) = file_id {
    let ws_id = ctx.active_workspace_id.get_untracked();
    let origin = ctx.effective_cloud_origin();
    let is_uuid = Uuid::parse_str(&fid).is_ok();
    let cloud_blob_url = if !ws_id.is_nil() && is_uuid {
      origin
        .as_deref()
        .map(|orig| build_file_blob_url(orig, ws_id, &fid, raw_name.as_deref()))
    } else {
      None
    };

    let cas_uri = cas_blob_file_uri(&fid);
    let is_cloud_stored = cloud_blob_url.is_some();
    let display_uri = cas_uri
      .clone()
      .or_else(|| cloud_blob_url.clone())
      .or(raw_url)
      .unwrap_or_default();
    let link_url = cloud_blob_url.or(cas_uri);

    return ResolvedImage {
      display_uri,
      link_url,
      file_id: Some(fid),
      external_url: None,
      file_name,
      caption,
      is_cloud_stored,
    };
  }

  if let Some(u) = raw_url {
    // Handle both server-relative `/api/workspaces/{ws}/files/{fid}/blob[/{name}]`
    // and full `<origin>/api/workspaces/{ws}/files/{fid}/blob[/{name}]` URLs
    // emitted by `GET /documents/{id}/markdown` or client Markdown persistence.
    if let Some(orig) = ctx.effective_cloud_origin() {
      let abs_opt = if u.starts_with("/api/") {
        Some(format!("{orig}{u}"))
      } else if is_http_url(&u) {
        Some(u.clone())
      } else {
        None
      };
      if let Some(abs) = abs_opt {
        if let Some((_ws, fid, parsed_name)) = parse_mica_blob_link(&abs, &orig) {
          let cas_uri = cas_blob_file_uri(&fid);
          let display_uri = cas_uri.unwrap_or_else(|| abs.clone());
          return ResolvedImage {
            display_uri,
            link_url: Some(abs),
            file_id: Some(fid),
            external_url: None,
            file_name: parsed_name.unwrap_or(file_name),
            caption,
            is_cloud_stored: true,
          };
        }
        if u.starts_with("/api/") {
          return ResolvedImage {
            display_uri: abs.clone(),
            link_url: Some(abs),
            file_id: None,
            external_url: None,
            file_name,
            caption,
            is_cloud_stored: true,
          };
        }
      }
    }

    let external = is_http_url(&u).then(|| u.clone());
    return ResolvedImage {
      display_uri: u.clone(),
      link_url: Some(u),
      file_id: None,
      external_url: external,
      file_name,
      caption,
      is_cloud_stored: false,
    };
  }

  ResolvedImage {
    display_uri: String::new(),
    link_url: None,
    file_id: None,
    external_url: None,
    file_name,
    caption,
    is_cloud_stored: false,
  }
}

fn infer_filename_from_url(url: &str) -> Option<String> {
  let without_q = url.split(['?', '#']).next()?;
  let seg = without_q.rsplit('/').next()?.trim();
  if seg.is_empty() || !seg.contains('.') {
    return None;
  }
  Some(seg.to_string())
}

/// Build `file_id -> permanent URL` map directly from a slice of `EditorBlock`s
/// so both synchronous exports (`MicaDocumentBridge::export_markdown`) and
/// off-thread debounced persistence (`export_blocks_markdown`) preserve working
/// cloud blob links (`/api/workspaces/{ws}/files/{fid}/blob/{name}`) and Desktop
/// CAS URIs (`file:///...`) instead of degrading uploaded images to bare filenames.
pub fn build_blocks_image_urls(
  blocks: &[EditorBlock],
  ws_id: Uuid,
  cloud_origin: Option<&str>,
) -> BTreeMap<String, String> {
  let mut out = BTreeMap::new();
  for block in blocks {
    if block.kind != "image" {
      continue;
    }
    let Some(fid) = block
      .data
      .get("file_id")
      .and_then(Value::as_str)
      .map(str::trim)
      .filter(|s| !s.is_empty())
    else {
      continue;
    };
    if out.contains_key(fid) {
      continue;
    }
    let name = block
      .data
      .get("name")
      .and_then(Value::as_str)
      .map(str::trim)
      .filter(|s| !s.is_empty());
    if !ws_id.is_nil()
      && Uuid::parse_str(fid).is_ok()
      && let Some(orig) = cloud_origin.filter(|s| !s.trim().is_empty())
    {
      out.insert(fid.to_string(), build_file_blob_url(orig, ws_id, fid, name));
    } else if let Some(uri) = cas_blob_file_uri(fid) {
      out.insert(fid.to_string(), uri);
    }
  }
  out
}

/// Build `file_id -> permanent URL` map for every image block in the active
/// document so `EditorCommand::CopySelection` / `CutSelection` emits working
/// Markdown & HTML image links (exact match with `v0.14.11`'s `_imageUrlCache`).
pub fn build_document_image_urls(
  bridge: &MicaDocumentBridge,
  ctx: &ImageContext,
) -> BTreeMap<String, String> {
  let mut out = BTreeMap::new();
  let ws_id = ctx.active_workspace_id.get_untracked();
  let origin = ctx.effective_cloud_origin();
  for (fid, name) in bridge.document_image_file_ids() {
    if !ws_id.is_nil()
      && Uuid::parse_str(&fid).is_ok()
      && let Some(orig) = origin.as_deref()
    {
      out.insert(fid.clone(), build_file_blob_url(orig, ws_id, &fid, name.as_deref()));
    } else if let Some(uri) = cas_blob_file_uri(&fid) {
      out.insert(fid, uri);
    }
  }
  out
}

// ── On-Device Content-Addressed Blob Mirror (Desktop CAS) ────────────────────

#[cfg(not(target_arch = "wasm32"))]
fn cas_blobs_dir() -> std::path::PathBuf {
  dirs_next::data_local_dir()
    .map(|d| d.join("mica").join("blobs"))
    .unwrap_or_else(|| std::env::temp_dir().join("mica-blobs"))
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitize_cas_key(key: &str) -> Option<&str> {
  let k = key.trim();
  if k.is_empty()
    || k.len() > 128
    || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
  {
    return None;
  }
  Some(k)
}

/// Store `bytes` under its lowercase SHA-256 hex digest in the Desktop CAS.
#[cfg(not(target_arch = "wasm32"))]
pub fn cas_put_blob(bytes: &[u8]) -> Option<String> {
  if bytes.is_empty() {
    return None;
  }
  let sha = sha256_hex(bytes);
  cas_put_blob_as(&sha, bytes).then_some(sha)
}

#[cfg(target_arch = "wasm32")]
pub fn cas_put_blob(_bytes: &[u8]) -> Option<String> {
  None
}

/// Mirror `bytes` under an explicit `key` (e.g. cloud `file_id` UUID or sha256)
/// in the Desktop CAS so offline sessions display cloud images immediately.
#[cfg(not(target_arch = "wasm32"))]
pub fn cas_put_blob_as(key: &str, bytes: &[u8]) -> bool {
  let Some(clean) = sanitize_cas_key(key) else {
    return false;
  };
  let dir = cas_blobs_dir();
  if std::fs::create_dir_all(&dir).is_err() {
    return false;
  }
  let path = dir.join(clean);
  if path.is_file() {
    return true;
  }
  let tmp = dir.join(format!("{clean}.{}.tmp", Uuid::new_v4()));
  if std::fs::write(&tmp, bytes).is_err() {
    let _ = std::fs::remove_file(&tmp);
    return false;
  }
  if std::fs::rename(&tmp, &path).is_err() {
    let _ = std::fs::remove_file(&tmp);
    return false;
  }
  true
}

#[cfg(target_arch = "wasm32")]
pub fn cas_put_blob_as(_key: &str, _bytes: &[u8]) -> bool {
  false
}

/// Return a `file:///...` URI if `key` is mirrored in the Desktop CAS.
#[cfg(not(target_arch = "wasm32"))]
pub fn cas_blob_file_uri(key: &str) -> Option<String> {
  let clean = sanitize_cas_key(key)?;
  let path = cas_blobs_dir().join(clean);
  path
    .is_file()
    .then(|| crate::clipboard_platform::path_to_file_uri(&path))
}

#[cfg(target_arch = "wasm32")]
pub fn cas_blob_file_uri(_key: &str) -> Option<String> {
  None
}

/// Read raw bytes of `key` from the Desktop CAS, if cached.
#[cfg(not(target_arch = "wasm32"))]
pub fn cas_read_blob(key: &str) -> Option<Vec<u8>> {
  let clean = sanitize_cas_key(key)?;
  let path = cas_blobs_dir().join(clean);
  std::fs::read(path).ok()
}

#[cfg(target_arch = "wasm32")]
pub fn cas_read_blob(_key: &str) -> Option<Vec<u8>> {
  None
}

// ── Async Upload, Insert, Replace, Re-host, Copy & Download ──────────────────

/// Upload (in Cloud mode) or CAS-store (in Local mode) raw image bytes and insert
/// a new `image` block into `bridge`.
pub fn insert_image_from_bytes(
  bridge: Rc<MicaDocumentBridge>,
  ctx: ImageContext,
  bytes: Vec<u8>,
  file_name: String,
  mime_hint: String,
  alt: String,
) {
  if bytes.is_empty() {
    return;
  }
  let mime = if mime_hint.starts_with("image/") {
    mime_hint
  } else {
    mime_for_image(&bytes, &file_name).to_string()
  };
  let clean_name = normalize_image_filename(&file_name, &mime);

  let is_cloud = ctx.is_cloud_signed_in.get_untracked();
  let ws_id = ctx.active_workspace_id.get_untracked();
  let doc_id = ctx.active_doc.get_untracked();

  if is_cloud && !ws_id.is_nil() {
    let conn_rc = ctx.cloud_connector.clone();
    let ctx_task = ctx.clone();
    ctx.spawn(async move {
      let mut conn = conn_rc.borrow().clone();
      let res = with_auto_refresh(&mut conn, |c| {
        let fname = clean_name.clone();
        let m = mime.clone();
        let data = bytes.clone();
        async move { c.upload_image(ws_id, &fname, &m, &data).await }
      })
      .await;
      match res {
        Ok((uploaded, rotated)) => {
          if rotated.is_some() {
            *conn_rc.borrow_mut() = conn;
          }
          cas_put_blob_as(&uploaded.file_id, &bytes);
          // Guard cross-await ownership: only mutate if still on the same doc/workspace.
          if ctx_task.active_doc.get_untracked() == doc_id
            && ctx_task.active_workspace_id.get_untracked() == ws_id
          {
            bridge.insert_image_block(
              Some(uploaded.file_id),
              None,
              Some(uploaded.name),
              &alt,
            );
          }
        }
        Err(err) => {
          // Offline Desktop fallback: persist in local CAS so pasted screenshots are never lost.
          if let Some(sha) = cas_put_blob(&bytes) {
            if ctx_task.active_doc.get_untracked() == doc_id {
              bridge.insert_image_block(Some(sha), None, Some(clean_name), &alt);
            }
            ctx_task.show_toast(format!("云端上传失败，已暂存到本地：{err}"));
          } else {
            ctx_task.show_toast(format!("图片上传失败：{err}"));
          }
        }
      }
    });
    return;
  }

  // Local workspace (Desktop): content-addressed CAS storage.
  if let Some(sha) = cas_put_blob(&bytes) {
    bridge.insert_image_block(Some(sha), None, Some(clean_name), &alt);
  } else if let Some(uri) =
    crate::clipboard_platform::persist_pasted_image(&bytes, &mime, &clean_name)
  {
    bridge.insert_image_block(None, Some(uri), Some(clean_name), &alt);
  }
}

/// Open the cross-platform image file picker (Desktop OS dialog or Web `<input type="file">`)
/// and insert the chosen image (uploading to cloud when signed in).
pub fn pick_and_insert_image(bridge: Rc<MicaDocumentBridge>, ctx: ImageContext) {
  let ctx_task = ctx.clone();
  ctx.spawn(async move {
    match crate::transfer::pick_import_file_bytes("Images", IMAGE_PICK_EXTS, false).await {
      Ok(files) => {
        if let Some(f) = files.into_iter().next() {
          let mime = mime_for_image(&f.bytes, &f.name).to_string();
          insert_image_from_bytes(bridge, ctx_task, f.bytes, f.name, mime, String::new());
        }
      }
      Err(err) => ctx_task.show_toast(format!("选择图片失败：{err}")),
    }
  });
}

/// Replace an existing image block's content by picking a new file from disk.
pub fn pick_and_replace_image(
  bridge: Rc<MicaDocumentBridge>,
  ctx: ImageContext,
  block_id: String,
) {
  let ctx_task = ctx.clone();
  ctx.spawn(async move {
    match crate::transfer::pick_import_file_bytes("Images", IMAGE_PICK_EXTS, false).await {
      Ok(files) => {
        if let Some(f) = files.into_iter().next() {
          let mime = mime_for_image(&f.bytes, &f.name).to_string();
          replace_image_from_bytes(bridge, ctx_task, block_id, f.bytes, f.name, mime).await;
        }
      }
      Err(err) => ctx_task.show_toast(format!("选择图片失败：{err}")),
    }
  });
}

/// Replace an existing image block with raw `bytes` (uploading in Cloud mode or CAS in Local mode).
pub async fn replace_image_from_bytes(
  bridge: Rc<MicaDocumentBridge>,
  ctx: ImageContext,
  block_id: String,
  bytes: Vec<u8>,
  file_name: String,
  mime: String,
) {
  if bytes.is_empty() {
    return;
  }
  let clean_name = normalize_image_filename(&file_name, &mime);
  let is_cloud = ctx.is_cloud_signed_in.get_untracked();
  let ws_id = ctx.active_workspace_id.get_untracked();
  let doc_id = ctx.active_doc.get_untracked();

  if is_cloud && !ws_id.is_nil() {
    let conn_rc = ctx.cloud_connector.clone();
    let mut conn = conn_rc.borrow().clone();
    let res = with_auto_refresh(&mut conn, |c| {
      let fname = clean_name.clone();
      let m = mime.clone();
      let data = bytes.clone();
      async move { c.upload_image(ws_id, &fname, &m, &data).await }
    })
    .await;
    match res {
      Ok((uploaded, rotated)) => {
        if rotated.is_some() {
          *conn_rc.borrow_mut() = conn;
        }
        cas_put_blob_as(&uploaded.file_id, &bytes);
        if ctx.active_doc.get_untracked() == doc_id
          && ctx.active_workspace_id.get_untracked() == ws_id
        {
          bridge.set_image_source_by_id(&block_id, Some(uploaded.file_id), None, Some(uploaded.name));
          ctx.show_toast("已替换图片");
        }
      }
      Err(err) => ctx.show_toast(format!("替换图片上传失败：{err}")),
    }
  } else if let Some(sha) = cas_put_blob(&bytes) {
    if ctx.active_doc.get_untracked() == doc_id {
      bridge.set_image_source_by_id(&block_id, Some(sha), None, Some(clean_name));
      ctx.show_toast("已替换图片");
    }
  } else if let Some(uri) =
    crate::clipboard_platform::persist_pasted_image(&bytes, &mime, &clean_name)
    && ctx.active_doc.get_untracked() == doc_id
  {
    bridge.set_image_source_by_id(&block_id, None, Some(uri), Some(clean_name));
    ctx.show_toast("已替换图片");
  }
}

/// Re-host an external `http(s)://` image block into Mica storage (server-side
/// `import-url` first, then client-side download + `upload_image` fallback — exact
/// match with `v0.14.11`'s two-stage ladder).
pub fn rehost_external_image_block(
  bridge: Rc<MicaDocumentBridge>,
  ctx: ImageContext,
  block_id: String,
  external_url: String,
) {
  let ws_id = ctx.active_workspace_id.get_untracked();
  let doc_id = ctx.active_doc.get_untracked();
  let is_cloud = ctx.is_cloud_signed_in.get_untracked();

  let ctx_task = ctx.clone();
  ctx.spawn(async move {
    if is_cloud && !ws_id.is_nil() {
      let conn_rc = ctx_task.cloud_connector.clone();
      let mut conn = conn_rc.borrow().clone();
      // 1. Server-side import-url (SSRF-guarded, reuses own workspace blob link).
      let server_res = with_auto_refresh(&mut conn, |c| {
        let u = external_url.clone();
        async move { c.import_image_url(ws_id, &u).await }
      })
      .await;
      if let Ok((uploaded, rotated)) = server_res {
        if rotated.is_some() {
          *conn_rc.borrow_mut() = conn;
        }
        if ctx_task.active_doc.get_untracked() == doc_id
          && ctx_task.active_workspace_id.get_untracked() == ws_id
        {
          bridge.set_image_source_by_id(
            &block_id,
            Some(uploaded.file_id),
            None,
            Some(uploaded.name),
          );
          ctx_task.show_toast("已转存到 Mica 存储");
        }
        return;
      }

      // 2. Client-side fallback: download from this device & upload.
      match download_url_bytes(&external_url, 20).await {
        Ok((bytes, ct)) if !bytes.is_empty() => {
          let name = infer_filename_from_url(&external_url).unwrap_or_else(|| "image.png".into());
          let mime = if ct.starts_with("image/") {
            ct.split(';').next().unwrap_or("image/png").trim().to_string()
          } else {
            mime_for_image(&bytes, &name).to_string()
          };
          replace_image_from_bytes(bridge, ctx_task, block_id, bytes, name, mime).await;
        }
        Ok(_) => ctx_task.show_toast("转存失败：外链返回空内容"),
        Err(err) => ctx_task.show_toast(format!("转存失败：{err}")),
      }
      return;
    }

    // Local workspace: download into Desktop CAS.
    match download_url_bytes(&external_url, 20).await {
      Ok((bytes, ct)) if !bytes.is_empty() => {
        let name = infer_filename_from_url(&external_url).unwrap_or_else(|| "image.png".into());
        let mime = if ct.starts_with("image/") {
          ct.split(';').next().unwrap_or("image/png").trim().to_string()
        } else {
          mime_for_image(&bytes, &name).to_string()
        };
        replace_image_from_bytes(bridge, ctx_task, block_id, bytes, name, mime).await;
      }
      Ok(_) => ctx_task.show_toast("转存失败：外链返回空内容"),
      Err(err) => ctx_task.show_toast(format!("转存失败：{err}")),
    }
  });
}

/// Fetch raw image bytes + MIME type for `resolved` (supporting CAS, `file://`,
/// `data:` base64, and `http(s)://` URLs; mirrors cloud files into Desktop CAS).
pub async fn load_resolved_image_bytes(
  resolved: &ResolvedImage,
) -> Result<(Vec<u8>, String), String> {
  if let Some(fid) = resolved.file_id.as_deref()
    && let Some(bytes) = cas_read_blob(fid)
    && !bytes.is_empty()
  {
    let mime = mime_for_image(&bytes, &resolved.file_name).to_string();
    return Ok((bytes, mime));
  }

  let uri = resolved.display_uri.trim();
  if uri.is_empty() {
    return Err("图片地址为空".to_string());
  }

  if let Some((mime, bytes)) = decode_data_uri(uri) {
    return Ok((bytes, mime));
  }

  #[cfg(not(target_arch = "wasm32"))]
  if let Some(path) = crate::clipboard_platform::file_uri_to_local_path(uri) {
    match std::fs::read(&path) {
      Ok(bytes) if !bytes.is_empty() => {
        let mime = mime_for_image(&bytes, &resolved.file_name).to_string();
        return Ok((bytes, mime));
      }
      Ok(_) | Err(_) => {
        // Fall through to `resolved.link_url` when a local CAS mirror disappeared
        // while a permanent cloud URL is still available.
        if !resolved.link_url.as_deref().is_some_and(is_http_url) {
          return Err("读取本地图片失败".to_string());
        }
      }
    }
  }

  let fetch_url = if is_http_url(uri) {
    Some(uri)
  } else {
    resolved.link_url.as_deref().filter(|u| is_http_url(u))
  };
  if let Some(http_uri) = fetch_url {
    let (bytes, ct) = download_url_bytes(http_uri, 25).await?;
    if bytes.is_empty() {
      return Err("图片内容为空".to_string());
    }
    if let Some(fid) = resolved.file_id.as_deref() {
      cas_put_blob_as(fid, &bytes);
    }
    let mime = if ct.starts_with("image/") {
      ct.split(';').next().unwrap_or("image/png").trim().to_string()
    } else {
      mime_for_image(&bytes, &resolved.file_name).to_string()
    };
    return Ok((bytes, mime));
  }

  Err("不支持的图片地址".to_string())
}

/// On WebAssembly, browser `navigator.clipboard.write(...)` requires synchronous
/// invocation inside the click/key gesture; `nsh.js`'s `writeClipboardImage` accepts
/// `data:` and same-origin `http(s)://` URLs directly and wraps the fetch inside
/// `new ClipboardItem({ "image/png": promise })` so Safari/Chrome never reject with
/// `NotAllowedError`.
#[cfg(target_arch = "wasm32")]
fn try_sync_web_clipboard_copy(resolved: &ResolvedImage, ctx: &ImageContext) -> bool {
  let uri = resolved.display_uri.trim();
  if uri.is_empty() {
    return false;
  }
  let can_direct_write = uri.starts_with("data:")
    || (is_http_url(uri)
      && (resolved.is_cloud_stored
        || mica_client::web_runtime::web_origin()
          .is_some_and(|orig| !orig.is_empty() && uri.starts_with(&orig))));
  if !can_direct_write {
    return false;
  }
  nsh_core::element::set_clipboard_image(uri.to_string());
  ctx.show_toast("已复制图片");
  true
}

/// Trigger an image bitmap copy from a user gesture (context menu, hover toolbar,
/// or `Ctrl+C`), spawning any async fetch under `ctx.owner` so closing the menu
/// never cancels the copy.
pub fn copy_image_from_user_action(resolved: ResolvedImage, ctx: ImageContext) {
  #[cfg(target_arch = "wasm32")]
  if try_sync_web_clipboard_copy(&resolved, &ctx) {
    return;
  }
  let ctx_task = ctx.clone();
  ctx.spawn(async move {
    let _ = copy_resolved_image_bitmap(&resolved, &ctx_task).await;
  });
}

/// Copy `resolved`'s bitmap onto the system clipboard (returns `true` on success).
pub async fn copy_resolved_image_bitmap(resolved: &ResolvedImage, ctx: &ImageContext) -> bool {
  match load_resolved_image_bytes(resolved).await {
    Ok((bytes, mime)) => {
      let ok = copy_image_bytes_to_clipboard(&bytes, &mime);
      ctx.show_toast(if ok { "已复制图片" } else { "复制图片失败" });
      ok
    }
    Err(err) => {
      ctx.show_toast(format!("读取图片失败：{err}"));
      false
    }
  }
}

/// Copy an image block's bitmap to the system clipboard, and delete the block
/// ONLY if the clipboard write succeeded (exact match with `v0.14.11`'s `_cutImage`).
pub fn cut_image_block(
  bridge: Rc<MicaDocumentBridge>,
  block_id: String,
  resolved: ResolvedImage,
  ctx: ImageContext,
) {
  #[cfg(target_arch = "wasm32")]
  if try_sync_web_clipboard_copy(&resolved, &ctx) {
    if let Some(idx) = find_block_index_by_id(&bridge, &block_id) {
      bridge.delete_block_at(idx);
    }
    return;
  }
  let doc_id = ctx.active_doc.get_untracked();
  let ctx_task = ctx.clone();
  ctx.spawn(async move {
    if !copy_resolved_image_bitmap(&resolved, &ctx_task).await {
      return;
    }
    if ctx_task.active_doc.get_untracked() != doc_id {
      return;
    }
    if let Some(idx) = find_block_index_by_id(&bridge, &block_id) {
      bridge.delete_block_at(idx);
    }
  });
}

/// Download / Save-As an image block to disk (OS Save-As dialog on Desktop,
/// browser Blob download on Web).
pub fn download_image_block(resolved: ResolvedImage, ctx: ImageContext) {
  let ctx_task = ctx.clone();
  ctx.spawn(async move {
    match load_resolved_image_bytes(&resolved).await {
      Ok((bytes, mime)) => {
        let name = normalize_image_filename(&resolved.file_name, &mime);
        match crate::transfer::save_bytes_with_dialog(&name, "Images", IMAGE_PICK_EXTS, &bytes).await
        {
          Ok(Some(_)) => ctx_task.show_toast(format!("已保存 {name}")),
          Ok(None) => {}
          Err(err) => ctx_task.show_toast(format!("保存图片失败：{err}")),
        }
      }
      Err(err) => ctx_task.show_toast(format!("下载图片失败：{err}")),
    }
  });
}

/// Desktop-only: read the current image (bitmap or copied image file) off the OS
/// clipboard and replace `block_id`.
#[cfg(not(target_arch = "wasm32"))]
pub fn paste_clipboard_image_over(
  bridge: Rc<MicaDocumentBridge>,
  block_id: String,
  ctx: ImageContext,
) {
  let ctx_task = ctx.clone();
  ctx.spawn(async move {
    let mut loaded: Option<(Vec<u8>, String, String)> = None;
    if let Some(clip) = nsh_core::element::clipboard_image().await {
      if let Some((mime, bytes)) = decode_data_uri(&clip.url) {
        loaded = Some((bytes, "pasted-image.png".into(), mime));
      } else if let Some(path) = crate::clipboard_platform::file_uri_to_local_path(&clip.url)
        && let Ok(bytes) = std::fs::read(&path)
      {
        let name = path
          .file_name()
          .and_then(|s| s.to_str())
          .unwrap_or("pasted-image.png")
          .to_string();
        let mime = mime_for_image(&bytes, &name).to_string();
        loaded = Some((bytes, name, mime));
      }
    }
    // Also support images copied as files in Explorer / Finder (`CF_HDROP`).
    if loaded.is_none() {
      for uri in nsh_core::element::clipboard_files().await {
        if let Some((file_uri, _)) =
          crate::clipboard_platform::image_uri_from_pasted_file_uri(&uri)
          && let Some(path) = crate::clipboard_platform::file_uri_to_local_path(&file_uri)
          && let Ok(bytes) = std::fs::read(&path)
          && !bytes.is_empty()
        {
          let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("pasted-image.png")
            .to_string();
          let mime = mime_for_image(&bytes, &name).to_string();
          loaded = Some((bytes, name, mime));
          break;
        }
      }
    }
    let Some((bytes, name, mime)) = loaded.filter(|(b, _, _)| !b.is_empty()) else {
      ctx_task.show_toast("剪贴板中没有图片");
      return;
    };
    replace_image_from_bytes(bridge, ctx_task, block_id, bytes, name, mime).await;
  });
}

#[cfg(target_arch = "wasm32")]
pub fn paste_clipboard_image_over(
  _bridge: Rc<MicaDocumentBridge>,
  _block_id: String,
  ctx: ImageContext,
) {
  ctx.show_toast("请直接按 Ctrl+V 粘贴图片");
}

/// If the editor's current selection is a collapsed caret (or single-block selection)
/// sitting directly on an `image` block, return `(block_index, block_id, ResolvedImage)`.
pub fn selected_single_image_block(
  bridge: &MicaDocumentBridge,
  ctx: &ImageContext,
) -> Option<(usize, String, ResolvedImage)> {
  let sel = bridge.selection()?;
  if sel.is_multi_node() {
    return None;
  }
  let idx = sel.focus_pos().node;
  let blk = bridge.block_at(idx)?;
  if blk.kind != "image" {
    return None;
  }
  let resolved = resolve_image_block(&blk, ctx);
  Some((idx, blk.id, resolved))
}

/// If `text` is a bare external image URL pasted while `rehost_images` is enabled,
/// recognize and upload/re-host it asynchronously (also reusing same-workspace Mica blob links).
pub fn try_handle_pasted_image_url(
  raw_text: &str,
  bridge: &Rc<MicaDocumentBridge>,
  ctx: &ImageContext,
) -> bool {
  let trimmed = raw_text.trim();
  if trimmed.contains('\n') || !is_http_url(trimmed) {
    return false;
  }
  let origin = ctx.effective_cloud_origin().unwrap_or_default();
  let ws_id = ctx.active_workspace_id.get_untracked();
  // Same-workspace Mica blob link: reuse `file_id` directly with zero re-upload!
  if ctx.is_cloud_signed_in.get_untracked()
    && !ws_id.is_nil()
    && let Some((link_ws, fid, name)) = parse_mica_blob_link(trimmed, &origin)
    && link_ws == ws_id
  {
    bridge.insert_image_block(Some(fid), None, name, "");
    return true;
  }
  false
}

fn find_block_index_by_id(bridge: &MicaDocumentBridge, block_id: &str) -> Option<usize> {
  let count = bridge.block_len();
  (0..count).find(|&i| bridge.block_at(i).is_some_and(|b| b.id == block_id))
}

fn normalize_image_filename(raw: &str, mime: &str) -> String {
  let trimmed = raw.trim();
  let ext = match mime {
    "image/jpeg" => "jpg",
    "image/gif" => "gif",
    "image/webp" => "webp",
    "image/bmp" => "bmp",
    "image/svg+xml" => "svg",
    _ => "png",
  };
  if trimmed.is_empty() {
    return format!("image.{ext}");
  }
  if trimmed.contains('.') {
    trimmed.to_string()
  } else {
    format!("{trimmed}.{ext}")
  }
}

// ── Image Block Renderer (1:1 v0.14.11 Parity) ───────────────────────────────

fn compute_display_dimensions(
  natural: Option<(f32, f32)>,
  explicit_w: Option<f32>,
  max_content_w: f32,
) -> (f32, f32) {
  let cap_w = max_content_w.clamp(220.0, 960.0);
  let max_h = 520.0_f32;
  if let Some((nw, nh)) = natural
    && nw > 1.0
    && nh > 1.0
  {
    if let Some(w) = explicit_w {
      let cw = w.clamp(80.0, cap_w);
      let ch = (cw * nh / nw).clamp(48.0, 960.0);
      return (cw, ch);
    }
    let scale = (cap_w / nw).min(max_h / nh).min(1.0);
    return ((nw * scale).clamp(80.0, cap_w), (nh * scale).clamp(48.0, max_h));
  }
  let w = explicit_w.unwrap_or(360.0).clamp(80.0, cap_w);
  (w, 220.0)
}

pub fn render_image_block_view(
  env: &BlockRenderEnv,
  at: Position,
  block: EditorBlock,
) -> impl IntoElement {
  let dark_mode = env.dark_mode;
  let bridge = env.bridge.clone();
  let lightbox = env.lightbox;
  let img_ctx = env.image_ctx.clone();
  let page_width = env.page_width;
  let marquee_selected = env.marquee_selected;
  let editor_focused = env.editor_focused;
  let caret_blink_on = env.caret_blink_on;
  let focused_code = env.focused_code_block;
  let focused_cell = env.focused_table_cell;
  let table_sel = env.table_selection;
  let table_drag = env.table_drag_anchor;
  let drag_anchor = env.drag_anchor;
  let blank_marquee_ready = env.body_blank_marquee_ready;
  let marquee_origin_page = env.marquee_origin_page;
  let sink_clear_rev = env.sink_clear_rev;
  let last_click = env.last_click;

  let hovered = signal(false);
  let copied_flash = signal(false);
  let copied_gen = signal(0u64);
  let load_failed = signal(false);

  // Live block index helper matching `block_views::live_index`.
  let live_idx = move || crate::block_views::live_index(at);

  let block_id = block.id.clone();
  let resolved = resolve_image_block(&block, &img_ctx);
  let align = block
    .data
    .get("align")
    .and_then(Value::as_str)
    .unwrap_or("left")
    .to_string();
  let stored_width = block
    .data
    .get("width")
    .and_then(Value::as_f64)
    .map(|v| (v as f32).clamp(80.0, 960.0));

  // Initial natural size from in-memory cache or local file header probe.
  let init_natural: Option<(f32, f32)> = img_ctx
    .natural_sizes
    .borrow()
    .get(&resolved.display_uri)
    .copied()
    .or_else(|| {
      probe_local_image_dimensions(&resolved.display_uri).map(|(w, h)| (w as f32, h as f32))
    });
  if let Some(sz) = init_natural {
    img_ctx
      .natural_sizes
      .borrow_mut()
      .insert(resolved.display_uri.clone(), sz);
  }
  let natural_size: Signal<Option<(f32, f32)>> = signal(init_natural);

  // Resize handle drag state (preview width locally, commit `data.width` on release).
  let resize_preview_w: Signal<Option<f32>> = signal(None);
  let resize_dragging = signal(false);
  let resize_start_x = signal(0.0_f32);
  let resize_start_w = signal(360.0_f32);

  // Whether this image block is currently selected (collapsed caret on block or range selection).
  let sel_sig = bridge.selection_signal();
  let is_selected = move || {
    let idx = live_idx();
    if let Some((lo, hi)) = marquee_selected.get()
      && idx >= lo
      && idx <= hi
    {
      return true;
    }
    sel_sig.get().is_some_and(|s| {
      let a = s.start_pos().node;
      let b = s.end_pos().node;
      idx >= a && idx <= b
    })
  };

  // Whether the editor's active collapsed caret currently sits on this image block
  // (shows a blinking vertical caret right next to the image's right edge so pressing
  // Enter naturally inserts a new paragraph line below).
  let is_caret_on_image = computed(move || {
    if !editor_focused.get() || marquee_selected.get().is_some() {
      return false;
    }
    let idx = live_idx();
    sel_sig
      .get()
      .is_some_and(|s| s.is_collapsed() && s.focus_pos().node == idx)
  });

  let br_outer = bridge.clone();
  let mut outer = view()
    .test_id("node-view-image")
    .w_full()
    .my_2()
    .cursor(Cursor::Text)
    .on_mouse_enter(move |_: &MouseEvent| hovered.set(true))
    .on_mouse_leave(move |_: &MouseEvent| hovered.set(false))
    .on_press_down(move |_: &PressEvent| {
      focused_code.set(None);
      focused_cell.set(None);
      table_sel.set(None);
      table_drag.set(None);
      marquee_selected.set(None);
      drag_anchor.set(None);
      blank_marquee_ready.set(false);
      marquee_origin_page.set(None);
      sink_clear_rev.update(|v| *v = v.wrapping_add(1));
      editor_focused.set(true);
      let idx = live_idx();
      br_outer.set_selection(DocSelection::caret(idx, 0));
      br_outer.request_focus();
    });
  outer = match align.as_str() {
    "center" => outer.items_center(),
    "right" => outer.items_end(),
    _ => outer.items_start(),
  };

  let is_external = resolved.is_external();
  let external_host = resolved.external_host();
  let display_uri = resolved.display_uri.clone();
  let caption_text = resolved.caption.clone();
  let caption_display = caption_text.clone();
  let caption_for_btn = caption_text.clone();
  let bid_cap = block_id.clone();
  let bid_ctx = block_id.clone();
  let resolved_ctx = resolved.clone();
  let ctx_menu_sig = img_ctx.context_menu;
  let cap_edit_sig = img_ctx.caption_edit;
  let br_ctx = bridge.clone();
  let br_resize = bridge.clone();

  let body: AnyElement = if display_uri.is_empty() {
    view()
      .w(px(360.0))
      .h(px(120.0))
      .items_center()
      .justify_center()
      .child(
        text("图片地址为空")
          .text_size(px(13.0))
          .text_color(move || palette(dark_mode.get()).text_muted),
      )
      .into_element()
  } else {
    let uri_img = display_uri.clone();
    let uri_zoom = display_uri.clone();
    let uri_cache = display_uri.clone();
    let alt_zoom = caption_text.clone();
    let sizes_cache = img_ctx.natural_sizes.clone();
    let br_click = bridge.clone();
    let fail_host = external_host.clone();

    button()
      .test_id("image-preview-btn")
      .accessibility_label("单击选中图片，双击放大查看")
      .relative()
      .cursor(Cursor::Pointer)
      .on_press(move || {
        marquee_selected.set(None);
        editor_focused.set(true);
        let idx = live_idx();
        br_click.set_selection(DocSelection::caret(idx, 0));
        br_click.request_focus();
        // Double-click detection (matching `v0.14.11`: single click selects, double click zooms).
        let now_ms = crate::touch_selection::now_ms();
        let (prev_idx, _prev_off, prev_t, prev_clicks) = last_click.get_untracked();
        let next_clicks = if prev_idx == idx && (now_ms - prev_t).abs() < 420.0 {
          prev_clicks.saturating_add(1)
        } else {
          1
        };
        last_click.set((idx, 0, now_ms, next_clicks));
        if next_clicks >= 2 {
          last_click.set((idx, 0, 0.0, 0));
          lightbox.set(Some(LightboxContent::Image {
            url: uri_zoom.clone(),
            alt: alt_zoom.clone(),
            natural_size: natural_size.get_untracked(),
          }));
        }
      })
      .child(
        image(ImageSrc::from(uri_img))
          .w(move || {
            let max_w = (page_width.get() - 48.0).clamp(240.0, 920.0);
            let w_opt = resize_preview_w.get().or(stored_width);
            let (w, _) = compute_display_dimensions(natural_size.get(), w_opt, max_w);
            px(w)
          })
          .h(move || {
            let max_w = (page_width.get() - 48.0).clamp(240.0, 920.0);
            let w_opt = resize_preview_w.get().or(stored_width);
            let (_, h) = compute_display_dimensions(natural_size.get(), w_opt, max_w);
            px(h)
          })
          .on_load(move |sz: &Size<DevicePixels>| {
            let w = sz.width.0 as f32;
            let h = sz.height.0 as f32;
            if w > 1.0 && h > 1.0 {
              load_failed.set(false);
              sizes_cache.borrow_mut().insert(uri_cache.clone(), (w, h));
              if natural_size.get_untracked() != Some((w, h)) {
                natural_size.set(Some((w, h)));
              }
            }
          })
          .on_error(move |_msg: &str| {
            load_failed.set(true);
          }),
      )
      // Graceful broken-image overlay when network/external image fails to load.
      .child(when(move || load_failed.get(), {
        let host_label = if is_external && !fail_host.is_empty() {
          format!("无法加载外链图片（{fail_host}）")
        } else {
          "图片加载失败".to_string()
        };
        move || {
          view()
            .absolute()
            .inset_0()
            .items_center()
            .justify_center()
            .bg(move || palette(dark_mode.get()).code_bg)
            .px_3()
            .child(
              text(host_label.clone())
                .text_size(px(12.5))
                .text_color(move || palette(dark_mode.get()).text_muted),
            )
        }
      }))
      // External-link top-left badge (exact parity with v0.14.11).
      .child(when(move || is_external, move || {
        view()
          .absolute()
          .top(px(8.0))
          .left(px(8.0))
          .px(px(6.0))
          .py(px(2.0))
          .rounded(px(4.0))
          .bg(move || palette(dark_mode.get()).backdrop)
          .pointer_events(PointerEvents::None)
          .child(
            text("外链")
              .text_size(px(11.0))
              .text_color(nsh_core::color::white()),
          )
      }))
      .into_element()
  };

  let url_expand = display_uri.clone();
  let alt_expand = caption_text.clone();
  let resolved_copy = resolved.clone();
  let ctx_copy = img_ctx.clone();
  let br_al = bridge.clone();
  let br_ac = bridge.clone();
  let br_ar = bridge.clone();
  let br_del = bridge.clone();
  let bid_tb_cap = block_id.clone();
  let cap_tb_init = caption_text.clone();

  let card = view()
    .test_id("image-card")
    .relative()
    .rounded(px(8.0))
    .overflow_hidden()
    .border_2()
    .border_color(move || {
      let p = palette(dark_mode.get());
      if is_selected() { p.accent } else { p.border_subtle }
    })
    .bg(move || palette(dark_mode.get()).surface_raised)
    .on_context_menu(move |e: &ContextMenuEvent| {
      let idx = live_idx();
      editor_focused.set(true);
      br_ctx.set_selection(DocSelection::caret(idx, 0));
      ctx_menu_sig.set(Some(ImageContextMenuState {
        block_id: bid_ctx.clone(),
        block_index: idx,
        resolved: resolved_ctx.clone(),
        x: e.page_position.x.0.max(8.0),
        y: e.page_position.y.0.max(8.0),
      }));
    })
    .child(body)
    // Hover / selected top-right floating toolbar (放大 / 复制 / 左 / 中 / 右 / 图注 / 删除).
    .child(when(move || hovered.get() || resize_dragging.get(), {
      let align_cur = align.clone();
      move || {
        let (b1, b2, b3, b4) = (br_al.clone(), br_ac.clone(), br_ar.clone(), br_del.clone());
        let u_exp = url_expand.clone();
        let a_exp = alt_expand.clone();
        let res_cp = resolved_copy.clone();
        let c_cp = ctx_copy.clone();
        let bid_c = bid_tb_cap.clone();
        let init_c = cap_tb_init.clone();
        let a_cur = align_cur.clone();
        view()
          .test_id("image-hover-toolbar")
          .absolute()
          .top(px(8.0))
          .right(px(8.0))
          .z_index(20)
          .flex_row()
          .items_center()
          .gap(px(2.0))
          .p_1()
          .rounded(px(6.0))
          .bg(move || palette(dark_mode.get()).surface_overlay)
          .border_1()
          .border_color(move || palette(dark_mode.get()).border_normal)
          .shadow_sm()
          .child(when(move || !u_exp.is_empty(), {
            let u2 = url_expand.clone();
            let a2 = a_exp.clone();
            move || {
              let u3 = u2.clone();
              let a3 = a2.clone();
              img_toolbar_chip("放大", false, false, dark_mode, move || {
                lightbox.set(Some(LightboxContent::Image {
                  url: u3.clone(),
                  alt: a3.clone(),
                  natural_size: natural_size.get_untracked(),
                }));
              })
            }
          }))
          .child(
            button()
              .test_id("image-copy-btn")
              .px_2()
              .py_1()
              .rounded(px(4.0))
              .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
              .on_press(move || {
                let r = res_cp.clone();
                let c = c_cp.clone();
                let next_gen = copied_gen.get_untracked().wrapping_add(1);
                copied_gen.set(next_gen);
                #[cfg(target_arch = "wasm32")]
                if try_sync_web_clipboard_copy(&r, &c) {
                  copied_flash.set(true);
                  c.spawn(async move {
                    timer(Duration::from_millis(1600)).await;
                    if copied_gen.get_untracked() == next_gen {
                      copied_flash.set(false);
                    }
                  });
                  return;
                }
                let c_task = c.clone();
                c.spawn(async move {
                  if copy_resolved_image_bitmap(&r, &c_task).await {
                    copied_flash.set(true);
                    timer(Duration::from_millis(1600)).await;
                    if copied_gen.get_untracked() == next_gen {
                      copied_flash.set(false);
                    }
                  }
                });
              })
              .child(
                text(move || if copied_flash.get() { "已复制" } else { "复制" })
                  .text_size(px(11.5))
                  .text_color(move || {
                    let p = palette(dark_mode.get());
                    if copied_flash.get() { p.accent } else { p.text_primary }
                  }),
              ),
          )
          .child(img_toolbar_chip("左", a_cur == "left", false, dark_mode, move || {
            b1.update_block_data_attr_at(live_idx(), "align", Value::String("left".into()));
          }))
          .child(img_toolbar_chip("中", a_cur == "center", false, dark_mode, move || {
            b2.update_block_data_attr_at(live_idx(), "align", Value::String("center".into()));
          }))
          .child(img_toolbar_chip("右", a_cur == "right", false, dark_mode, move || {
            b3.update_block_data_attr_at(live_idx(), "align", Value::String("right".into()));
          }))
          .child(img_toolbar_chip("图注", !init_c.is_empty(), false, dark_mode, move || {
            cap_edit_sig.set(Some(ImageCaptionEditState {
              block_id: bid_c.clone(),
              block_index: live_idx(),
              initial: init_c.clone(),
            }));
          }))
          .child(img_toolbar_chip("删除", false, true, dark_mode, move || {
            b4.delete_block_at(live_idx());
          }))
      }
    }))
    // Right-edge draggable resize handle (exact parity with v0.14.11).
    .child(when(move || (hovered.get() || resize_dragging.get()) && !display_uri.is_empty(), move || {
      let br_r = br_resize.clone();
      view()
        .test_id("image-resize-handle")
        .absolute()
        .right(px(4.0))
        .top(relative(0.5))
        .mt(px(-20.0))
        .w(px(8.0))
        .h(px(40.0))
        .rounded(px(4.0))
        .bg(move || palette(dark_mode.get()).backdrop)
        .border_1()
        .border_color(nsh_core::color::white())
        .cursor(Cursor::ColResize)
        .z_index(21)
        .claim_touches(TouchClaim::on_start())
        .keep_touches(true)
        .on_responder_grant(move |e: &ResponderEvent| {
          let max_w = (page_width.get_untracked() - 48.0).clamp(240.0, 920.0);
          let (cur_w, _) = compute_display_dimensions(
            natural_size.get_untracked(),
            resize_preview_w.get_untracked().or(stored_width),
            max_w,
          );
          resize_dragging.set(true);
          resize_start_x.set(f32::from(e.page.x));
          resize_start_w.set(cur_w);
          resize_preview_w.set(Some(cur_w));
        })
        .on_responder_move(move |e: &ResponderEvent| {
          if !resize_dragging.get_untracked() {
            return;
          }
          let max_w = (page_width.get_untracked() - 48.0).clamp(240.0, 920.0);
          let dx = f32::from(e.page.x) - resize_start_x.get_untracked();
          let next_w = (resize_start_w.get_untracked() + dx).clamp(80.0, max_w);
          resize_preview_w.set(Some(next_w));
        })
        .on_responder_release(move |_: &ResponderEvent| {
          resize_dragging.set(false);
          if let Some(final_w) = resize_preview_w.get_untracked() {
            let rounded = final_w.round() as f64;
            br_r.update_block_data_attr_at(live_idx(), "width", Value::from(rounded));
          }
        })
    }));

  let card_with_caret = view()
    .flex_row()
    .items_end()
    .child(card)
    .child(when(move || is_caret_on_image.get(), move || {
      view()
        .test_id("image-trailing-caret")
        .ml(px(4.0))
        .mb(px(2.0))
        .w(px(2.0))
        .h(px(22.0))
        .rounded(px(1.0))
        .pointer_events(PointerEvents::None)
        .bg(move || {
          if caret_blink_on.get() {
            palette(dark_mode.get()).accent
          } else {
            crate::block_views::TRANSPARENT_RGBA
          }
        })
    }));

  outer.child(card_with_caret).child(when(
    move || !caption_display.trim().is_empty(),
    move || {
      let cap_label = caption_for_btn.clone();
      let bid_c = bid_cap.clone();
      let cap_init = caption_for_btn.clone();
      button()
        .pt_1()
        .on_press(move || {
          cap_edit_sig.set(Some(ImageCaptionEditState {
            block_id: bid_c.clone(),
            block_index: live_idx(),
            initial: cap_init.clone(),
          }));
        })
        .child(
          text(cap_label)
            .text_size(px(12.0))
            .text_color(move || palette(dark_mode.get()).text_muted),
        )
    },
  ))
}

fn img_toolbar_chip(
  label: &'static str,
  active: bool,
  danger: bool,
  dark_mode: Signal<bool>,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .px_2()
    .py_1()
    .rounded(px(4.0))
    .bg(move || {
      let p = palette(dark_mode.get());
      if active { p.accent_soft } else { crate::block_views::TRANSPARENT_RGBA }
    })
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_press)
    .child(text(label).text_size(px(11.5)).text_color(move || {
      let p = palette(dark_mode.get());
      if danger {
        p.danger
      } else if active {
        p.accent
      } else {
        p.text_primary
      }
    }))
}

// ── Top-Level Image Context Menu, Caption Modal & Toast ──────────────────────

pub fn render_image_context_menu_overlay(
  ctx: ImageContext,
  bridge: Rc<MicaDocumentBridge>,
  lightbox: Signal<Option<LightboxContent>>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let menu_sig = ctx.context_menu;
  when(move || menu_sig.get().is_some(), {
    let ctx_o = ctx.clone();
    let br_o = bridge.clone();
    move || {
      let Some(state) = menu_sig.get() else {
        return view().into_element();
      };
      let dismiss = move || menu_sig.set(None);
      let status_label = if state.resolved.is_external() {
        format!("外链 · {}", state.resolved.external_host())
      } else if state.resolved.is_cloud_stored {
        "Mica 云端公开存储".to_string()
      } else {
        "本地存储".to_string()
      };
      let is_ext = state.resolved.is_external();
      let ext_url = state.resolved.external_url.clone();
      let link_opt = state.resolved.link_url.clone();
      let display_uri = state.resolved.display_uri.clone();
      let caption_cur = state.resolved.caption.clone();
      let bid = state.block_id.clone();
      let bidx = state.block_index;

      let mut menu = view()
        .test_id("image-context-menu")
        .absolute()
        .left(px(state.x.clamp(8.0, 1100.0)))
        .top(px(state.y.clamp(8.0, 560.0)))
        .w(px(210.0))
        .py_1()
        .rounded(px(8.0))
        .border_1()
        .border_color(move || palette(dark_mode.get()).border_normal)
        .bg(move || palette(dark_mode.get()).surface_overlay)
        .shadow_md()
        .child(
          view()
            .w_full()
            .px_3()
            .py(px(6.0))
            .border_b_1()
            .border_color(move || palette(dark_mode.get()).border_subtle)
            .child(
              text(status_label)
                .text_size(px(11.5))
                .text_color(move || palette(dark_mode.get()).text_faint),
            ),
        );

      // 1. 全屏查看
      if !display_uri.is_empty() {
        let u = display_uri.clone();
        let a = caption_cur.clone();
        let sizes = ctx_o.natural_sizes.clone();
        menu = menu.child(ctx_menu_row("img-menu-fullscreen", "全屏查看", false, dark_mode, move || {
          menu_sig.set(None);
          let nat = sizes.borrow().get(&u).copied();
          lightbox.set(Some(LightboxContent::Image {
            url: u.clone(),
            alt: a.clone(),
            natural_size: nat,
          }));
        }));
      }

      // 2. 编辑图注
      {
        let cap_sig = ctx_o.caption_edit;
        let bid_c = bid.clone();
        let init_c = caption_cur.clone();
        menu = menu.child(ctx_menu_row("img-menu-caption", "编辑图注", false, dark_mode, move || {
          menu_sig.set(None);
          cap_sig.set(Some(ImageCaptionEditState {
            block_id: bid_c.clone(),
            block_index: bidx,
            initial: init_c.clone(),
          }));
        }));
      }

      // 3. 替换图片…
      {
        let br_r = br_o.clone();
        let ctx_r = ctx_o.clone();
        let bid_r = bid.clone();
        menu = menu.child(ctx_menu_row("img-menu-replace", "替换图片…", false, dark_mode, move || {
          menu_sig.set(None);
          pick_and_replace_image(br_r.clone(), ctx_r.clone(), bid_r.clone());
        }));
      }

      // 4. 转存到 Mica (external only)
      if is_ext && let Some(eu) = ext_url {
        let br_h = br_o.clone();
        let ctx_h = ctx_o.clone();
        let bid_h = bid.clone();
        menu = menu.child(ctx_menu_row("img-menu-rehost", "转存到 Mica", false, dark_mode, move || {
          menu_sig.set(None);
          rehost_external_image_block(br_h.clone(), ctx_h.clone(), bid_h.clone(), eu.clone());
        }));
      }

      menu = menu.child(ctx_menu_divider(dark_mode));

      // 5. 复制图片 (bitmap)
      {
        let res_cp = state.resolved.clone();
        let ctx_cp = ctx_o.clone();
        menu = menu.child(ctx_menu_row("img-menu-copy", "复制图片", false, dark_mode, move || {
          menu_sig.set(None);
          copy_image_from_user_action(res_cp.clone(), ctx_cp.clone());
        }));
      }

      // 6. 剪切图片
      {
        let res_ct = state.resolved.clone();
        let ctx_ct = ctx_o.clone();
        let br_ct = br_o.clone();
        let bid_ct = bid.clone();
        menu = menu.child(ctx_menu_row("img-menu-cut", "剪切图片", false, dark_mode, move || {
          menu_sig.set(None);
          cut_image_block(br_ct.clone(), bid_ct.clone(), res_ct.clone(), ctx_ct.clone());
        }));
      }

      // 7. 粘贴替换 (Desktop only, matching v0.14.11)
      #[cfg(not(target_arch = "wasm32"))]
      {
        let br_pr = br_o.clone();
        let ctx_pr = ctx_o.clone();
        let bid_pr = bid.clone();
        menu = menu.child(ctx_menu_row("img-menu-paste-replace", "粘贴替换", false, dark_mode, move || {
          menu_sig.set(None);
          paste_clipboard_image_over(br_pr.clone(), bid_pr.clone(), ctx_pr.clone());
        }));
      }

      // 8. 复制链接 / 在浏览器打开
      if let Some(link) = link_opt {
        let l1 = link.clone();
        let ctx_l = ctx_o.clone();
        menu = menu.child(ctx_menu_row("img-menu-copy-link", "复制链接", false, dark_mode, move || {
          menu_sig.set(None);
          set_clipboard(l1.clone());
          ctx_l.show_toast(if is_ext {
            "已复制原图链接"
          } else {
            "已复制图片公开链接"
          });
        }));
        if is_http_url(&link) {
          let l2 = link;
          menu = menu.child(ctx_menu_row("img-menu-open-browser", "在浏览器打开", false, dark_mode, move || {
            menu_sig.set(None);
            drop(open_url(&l2));
          }));
        }
      }

      // 9. 下载图片
      {
        let res_dl = state.resolved.clone();
        let ctx_dl = ctx_o.clone();
        menu = menu.child(ctx_menu_row("img-menu-download", "下载图片", false, dark_mode, move || {
          menu_sig.set(None);
          download_image_block(res_dl.clone(), ctx_dl.clone());
        }));
      }

      menu = menu.child(ctx_menu_divider(dark_mode));

      // 10. 删除
      {
        let br_d = br_o.clone();
        let bid_d = bid;
        menu = menu.child(ctx_menu_row("img-menu-delete", "删除", true, dark_mode, move || {
          menu_sig.set(None);
          if let Some(idx) = find_block_index_by_id(&br_d, &bid_d) {
            br_d.delete_block_at(idx);
          }
        }));
      }

      overlay()
        .child(
          view()
            .absolute()
            .inset_0()
            .z_index(80)
            .child(
              button()
                .accessibility_label("关闭图片菜单")
                .absolute()
                .inset_0()
                .on_press(dismiss),
            )
            .child(menu),
        )
        .into_element()
    }
  })
}

fn ctx_menu_row(
  tid: &'static str,
  label: &'static str,
  danger: bool,
  dark_mode: Signal<bool>,
  on_press: impl Fn() + 'static,
) -> impl IntoElement {
  button()
    .test_id(tid)
    .w_full()
    .px_3()
    .py(px(6.0))
    .items_start()
    .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
    .on_press(on_press)
    .child(text(label).text_size(px(12.5)).text_color(move || {
      let p = palette(dark_mode.get());
      if danger { p.danger } else { p.text_primary }
    }))
}

fn ctx_menu_divider(dark_mode: Signal<bool>) -> impl IntoElement {
  view()
    .w_full()
    .h(px(1.0))
    .my_1()
    .bg(move || palette(dark_mode.get()).border_subtle)
}

pub fn render_image_caption_modal(
  ctx: ImageContext,
  bridge: Rc<MicaDocumentBridge>,
  dark_mode: Signal<bool>,
) -> impl IntoElement {
  let cap_sig = ctx.caption_edit;
  when(move || cap_sig.get().is_some(), {
    let br_o = bridge.clone();
    move || {
      let Some(st) = cap_sig.get() else {
        return view().into_element();
      };
      let draft = signal(st.initial.clone());
      let bid_save = st.block_id.clone();
      let bidx_fallback = st.block_index;
      let br_save = br_o.clone();
      let commit = move || {
        let val = draft.get_untracked().trim().to_string();
        let target_idx = find_block_index_by_id(&br_save, &bid_save).unwrap_or(bidx_fallback);
        let attr = if val.is_empty() { Value::Null } else { Value::String(val) };
        br_save.update_block_data_attr_at(target_idx, "caption", attr);
        cap_sig.set(None);
      };
      let commit_submit = commit.clone();
      let close = move || cap_sig.set(None);

      overlay()
        .child(
          view()
            .absolute()
            .inset_0()
            .z_index(85)
            .items_center()
            .justify_center()
            .bg(move || palette(dark_mode.get()).backdrop)
            .child(
              button()
                .accessibility_label("取消编辑图注")
                .absolute()
                .inset_0()
                .on_press(close),
            )
            .child(
              view()
                .w(px(380.0))
                .p_4()
                .gap_3()
                .rounded(px(10.0))
                .border_1()
                .border_color(move || palette(dark_mode.get()).border_normal)
                .bg(move || palette(dark_mode.get()).surface_overlay)
                .shadow_md()
                .z_index(86)
                .child(
                  text("编辑图片说明")
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || palette(dark_mode.get()).text_primary),
                )
                .child(
                  input()
                    .test_id("image-caption-input")
                    .auto_focus(true)
                    .placeholder("输入图注内容（留空清除）")
                    .submit_behavior(SubmitBehavior::Submit)
                    .w_full()
                    .h(px(34.0))
                    .px_3()
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(move || palette(dark_mode.get()).border_normal)
                    .bg(move || palette(dark_mode.get()).surface_base)
                    .text_size(px(13.0))
                    .text_color(move || palette(dark_mode.get()).text_primary)
                    .value(move || draft.get())
                    .on_change(move |v: &str| draft.set(v.to_owned()))
                    .on_submit(move |_: &str| commit_submit()),
                )
                .child(
                  view()
                    .w_full()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .child(
                      button()
                        .px_3()
                        .py_1()
                        .rounded(px(6.0))
                        .hover(move |s| s.bg(palette(dark_mode.get_untracked()).surface_hover))
                        .on_press(close)
                        .child(
                          text("取消")
                            .text_size(px(12.5))
                            .text_color(move || palette(dark_mode.get()).text_muted),
                        ),
                    )
                    .child(
                      button()
                        .test_id("image-caption-save")
                        .px_3()
                        .py_1()
                        .rounded(px(6.0))
                        .bg(move || palette(dark_mode.get()).accent)
                        .on_press(commit)
                        .child(text("保存").text_size(px(12.5)).text_color(nsh_core::color::white())),
                    ),
                ),
            ),
        )
        .into_element()
    }
  })
}

pub fn render_image_toast(ctx: ImageContext, dark_mode: Signal<bool>) -> impl IntoElement {
  let toast_sig = ctx.toast_state;
  when(move || toast_sig.get().is_some(), move || {
    view()
      .absolute()
      .bottom(px(20.0))
      .left_0()
      .right_0()
      .items_center()
      .z_index(75)
      .pointer_events(PointerEvents::None)
      .child(
        view()
          .test_id("image-action-toast")
          .px_4()
          .py_2()
          .max_w(px(420.0))
          .rounded(px(8.0))
          .items_center()
          .justify_center()
          .bg(move || palette(dark_mode.get()).surface_overlay)
          .border_1()
          .border_color(move || palette(dark_mode.get()).border_normal)
          .shadow_md()
          .child(
            text(move || toast_sig.get().map(|(m, _)| m).unwrap_or_default())
              .text_size(px(12.5))
              .text_color(move || palette(dark_mode.get()).text_primary),
          ),
      )
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn with_runtime(f: impl FnOnce()) {
    let rt = nsh_core::element::Runtime::default();
    let _g = rt.enter();
    f();
  }

  #[test]
  fn resolve_cloud_file_id_builds_permanent_blob_url() {
    with_runtime(|| {
      let ws = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
      let fid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
      let ctx = ImageContext::new(
        signal(true),
        signal("https://mica.cloudcele.com".to_string()),
        signal(ws),
        signal(None),
        Rc::new(RefCell::new(CloudConnector::new(Default::default()))),
        signal(true),
      );
      let mut blk = EditorBlock::new("img-1", "image", "");
      blk.data = json!({ "file_id": fid, "name": "cx8-label.png", "caption": "B300 标签" });

      let resolved = resolve_image_block(&blk, &ctx);
      assert_eq!(resolved.file_id.as_deref(), Some(fid));
      assert!(resolved.is_cloud_stored);
      assert!(!resolved.is_external());
      assert_eq!(resolved.caption, "B300 标签");
      assert_eq!(
        resolved.link_url.as_deref(),
        Some("https://mica.cloudcele.com/api/workspaces/11111111-2222-3333-4444-555555555555/files/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/blob/cx8-label.png")
      );
      assert!(!resolved.display_uri.is_empty());
    });
  }

  #[test]
  fn resolve_server_relative_blob_path_prepends_origin_and_recognizes_file_id() {
    with_runtime(|| {
      let ws = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
      let fid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
      let ctx = ImageContext::new(
        signal(true),
        signal("https://mica.cloudcele.com".to_string()),
        signal(ws),
        signal(None),
        Rc::new(RefCell::new(CloudConnector::new(Default::default()))),
        signal(true),
      );
      let mut blk = EditorBlock::new("img-rel", "image", "");
      blk.data = json!({
        "url": format!("/api/workspaces/{ws}/files/{fid}/blob/cx8.png")
      });
      let resolved = resolve_image_block(&blk, &ctx);
      assert!(resolved.is_cloud_stored);
      assert!(!resolved.is_external());
      assert_eq!(resolved.file_id.as_deref(), Some(fid));
      assert_eq!(
        resolved.link_url.as_deref(),
        Some("https://mica.cloudcele.com/api/workspaces/11111111-2222-3333-4444-555555555555/files/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/blob/cx8.png")
      );
      assert!(
        resolved.display_uri.starts_with("https://mica.cloudcele.com/")
          || resolved.display_uri.starts_with("file:///")
      );
    });
  }

  #[test]
  fn resolve_external_url_marks_external_and_extracts_host() {
    with_runtime(|| {
      let ctx = ImageContext::offline_default();
      let mut blk = EditorBlock::new("img-2", "image", "alt caption");
      blk.data = json!({ "url": "https://cdn.example.com/pics/rack.jpg?v=1" });

      let resolved = resolve_image_block(&blk, &ctx);
      assert!(resolved.is_external());
      assert!(!resolved.is_cloud_stored);
      assert_eq!(resolved.external_host(), "cdn.example.com");
      assert_eq!(resolved.file_name, "rack.jpg");
      assert_eq!(resolved.caption, "alt caption");
    });
  }

  #[test]
  fn build_document_image_urls_maps_file_ids_for_selection_copy() {
    with_runtime(|| {
      let ws = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
      let fid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
      let ctx = ImageContext::new(
        signal(true),
        signal("https://mica.cloudcele.com".to_string()),
        signal(ws),
        signal(None),
        Rc::new(RefCell::new(CloudConnector::new(Default::default()))),
        signal(true),
      );
      let bridge = MicaDocumentBridge::from_markdown("");
      bridge.insert_image_block(Some(fid.to_string()), None, Some("photo.png".into()), "图1");
      let urls = build_document_image_urls(&bridge, &ctx);
      assert_eq!(
        urls.get(fid).map(String::as_str),
        Some("https://mica.cloudcele.com/api/workspaces/11111111-2222-3333-4444-555555555555/files/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/blob/photo.png")
      );

      // Selection copy with resolved image URLs must emit a working Markdown image link!
      bridge.select_all();
      let payload = bridge.copy_selection_with_images(urls).expect("clipboard payload");
      assert!(
        payload.markdown.contains("/files/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/blob/photo.png"),
        "markdown copy must resolve file_id to permanent blob URL, got: {}",
        payload.markdown
      );
      assert!(
        payload.html.contains("/files/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/blob/photo.png"),
        "html copy must resolve file_id to permanent blob URL, got: {}",
        payload.html
      );
    });
  }

  #[test]
  fn set_image_source_by_id_preserves_width_align_and_caption() {
    with_runtime(|| {
      let bridge = MicaDocumentBridge::from_markdown("");
      bridge.insert_image_block(
        None,
        Some("https://cdn.example.com/old.png".into()),
        Some("old.png".into()),
        "",
      );
      let img_idx = (0..bridge.block_len())
        .find(|&i| bridge.block_at(i).is_some_and(|b| b.kind == "image"))
        .expect("image block exists");
      bridge.update_block_data_attr_at(img_idx, "align", Value::String("center".into()));
      bridge.update_block_data_attr_at(img_idx, "width", Value::from(420.0));
      bridge.update_block_data_attr_at(img_idx, "caption", Value::String("保留图注".into()));

      let bid = bridge.block_at(img_idx).unwrap().id;
      assert!(bridge.set_image_source_by_id(
        &bid,
        Some("11111111-2222-3333-4444-555555555555".into()),
        None,
        Some("rehosted.png".into()),
      ));
      let updated = bridge.block_at(img_idx).unwrap();
      assert_eq!(updated.data.get("file_id").and_then(Value::as_str), Some("11111111-2222-3333-4444-555555555555"));
      assert!(updated.data.get("url").is_none());
      assert_eq!(updated.data.get("align").and_then(Value::as_str), Some("center"));
      assert_eq!(updated.data.get("width").and_then(Value::as_f64), Some(420.0));
      assert_eq!(updated.data.get("caption").and_then(Value::as_str), Some("保留图注"));
    });
  }

  #[test]
  fn resolve_full_mica_blob_url_stays_cloud_stored_not_external() {
    with_runtime(|| {
      let ws = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
      let fid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
      let ctx = ImageContext::new(
        signal(true),
        signal("https://mica.cloudcele.com".to_string()),
        signal(ws),
        signal(None),
        Rc::new(RefCell::new(CloudConnector::new(Default::default()))),
        signal(true),
      );
      let mut blk = EditorBlock::new("img-3", "image", "");
      blk.data = json!({
        "url": format!("https://mica.cloudcele.com/api/workspaces/{ws}/files/{fid}/blob/arch.png")
      });

      let resolved = resolve_image_block(&blk, &ctx);
      assert!(!resolved.is_external(), "own Mica blob URL must not be flagged as external");
      assert!(resolved.is_cloud_stored);
      assert_eq!(resolved.file_id.as_deref(), Some(fid));
      assert_eq!(resolved.file_name, "arch.png");

      // build_blocks_image_urls preserves permanent blob hrefs for off-thread markdown export.
      let mut blk_fid = EditorBlock::new("img-4", "image", "");
      blk_fid.data = json!({ "file_id": fid, "name": "arch.png" });
      let map = build_blocks_image_urls(&[blk_fid], ws, Some("https://mica.cloudcele.com"));
      assert_eq!(
        map.get(fid).map(String::as_str),
        Some("https://mica.cloudcele.com/api/workspaces/11111111-2222-3333-4444-555555555555/files/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/blob/arch.png")
      );
    });
  }
}
