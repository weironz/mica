use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, header},
  response::{IntoResponse, Response},
};
use mica_app_core::{
  AppState,
  documents::export_html,
  store::{self},
};
use mica_infra::ApiResult;
use mica_markdown::SAFE_HTML_CSP;
use serde::Serialize;
use uuid::Uuid;

use super::export::inline_blob_hrefs;
use super::*;
use crate::routes::auth::user_id_from_headers;

// ── Public sharing (publish a page to a /s/{token} URL) ──────────────────────

#[derive(Debug, Serialize)]
pub struct ShareResponse {
  /// Whether the document currently has an active public link.
  shared: bool,
  /// The share token when shared; the client composes `{origin}/s/{token}`.
  #[serde(skip_serializing_if = "Option::is_none")]
  token: Option<String>,
}

/// `GET /api/workspaces/{ws}/documents/{id}/share` — current share status.
pub async fn get_share(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ShareResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_member(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let share = store::fetch_active_share_for_doc(&state.db, workspace_id, document_id).await?;
  Ok(Json(ShareResponse {
    shared: share.is_some(),
    token: share.map(|s| s.token),
  }))
}

/// `POST …/share` — publish (create-or-return the active share). Idempotent:
/// re-publishing returns the SAME link.
pub async fn create_share(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ShareResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  let share = store::create_or_get_share(&state.db, workspace_id, document_id, user_id).await?;
  Ok(Json(ShareResponse {
    shared: true,
    token: Some(share.token),
  }))
}

/// `DELETE …/share` — unpublish (soft-revoke). The public link 404s at once.
pub async fn delete_share(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, document_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ShareResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  ensure_workspace_editor(&state.db, workspace_id, user_id).await?;
  ensure_document_in_workspace(&state.db, workspace_id, document_id).await?;
  store::revoke_share(&state.db, workspace_id, document_id).await?;
  Ok(Json(ShareResponse {
    shared: false,
    token: None,
  }))
}

/// `GET /s/{token}` — the PUBLIC read-only page. No auth: the unguessable token
/// is the only credential, and it is re-checked (`revoked_at IS NULL`) on every
/// hit so revocation is instant. A missing/revoked/wrong token and a private
/// doc all return the SAME 404 — the page never reveals that a document exists.
/// Renders the server-side HTML (Rust-first data-plane) wrapped in a minimal
/// shell; images resolve through the public blob capability URLs.
pub async fn public_share_page(
  State(state): State<AppState>,
  Path(token): Path<String>,
) -> Response {
  let not_found = || {
    (
      axum::http::StatusCode::NOT_FOUND,
      [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
      "<!doctype html><meta charset=utf-8><title>Not found</title><p>This page is not available.</p>",
    )
      .into_response()
  };

  let Ok(Some(share)) = store::fetch_share_by_token(&state.db, &token).await else {
    return not_found();
  };
  // The share row alone is not enough: a token can outlive its document. If the
  // view was trashed (`is_deleted`) or purged, `fetch_document_view` (which
  // filters `is_deleted = false`) returns `None`, and we must 404 rather than
  // keep serving the still-present `current_payload` of deleted content.
  let Ok(Some(view)) = fetch_document_view(&state.db, share.workspace_id, share.document_id).await
  else {
    return not_found();
  };
  let Ok(Some(mut payload)) = store::current_payload(&state.db, share.document_id).await else {
    return not_found();
  };
  inline_blob_hrefs(&mut payload.blocks, share.workspace_id);
  let Ok(body) = export_html(&payload) else {
    return not_found();
  };
  let title = view.name;

  let html = render_share_shell(&title, &body, share.allow_indexing);
  (
    [
      (header::CONTENT_TYPE, "text/html; charset=utf-8"),
      // The share page is server-rendered static HTML that needs no JS. It
      // carries THE SAME policy the HTML export embeds as a <meta>
      // (`SAFE_HTML_CSP` — one constant, so the two surfaces cannot drift):
      // no `script-src` falls back to `default-src 'none'`, which neutralizes
      // inline `<script>` AND `on*` event handlers (e.g. `<img onerror>`) that
      // survive the GFM tagfilter in raw-HTML content — the storage-XSS ->
      // token-theft vector. `form-action 'none'` blocks same-origin form
      // phishing, `base-uri 'none'` blocks relative-link hijacking, and
      // `img-src`/`font-src`/`style-src` match what `render_share_shell`
      // actually uses (inline CSS, blob/data images, system fonts).
      (header::CONTENT_SECURITY_POLICY, SAFE_HTML_CSP),
    ],
    html,
  )
    .into_response()
}

/// Wrap the HTML export fragment in a standalone, readable page. `noindex`
/// unless the share opted into indexing, so a shared page is not silently
/// crawlable.
fn render_share_shell(title: &str, body_html: &str, allow_indexing: bool) -> String {
  let safe_title = escape_html_min(title);
  let robots = if allow_indexing {
    ""
  } else {
    "<meta name=\"robots\" content=\"noindex\">"
  };
  format!(
    "<!doctype html>\n<html lang=\"zh\">\n<head>\n<meta charset=\"utf-8\">\n\
     <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
     {robots}\n<title>{safe_title}</title>\n<style>\n\
     body{{max-width:1160px;margin:2.5rem auto;padding:0 1.5rem;\
     font-family:-apple-system,BlinkMacSystemFont,'Segoe UI','Microsoft YaHei',\
     'PingFang SC',sans-serif;line-height:1.7;color:#1f2328;}}\n\
     img{{max-width:100%;height:auto;border-radius:6px;}}\n\
     pre{{background:#f6f8fa;padding:1rem;border-radius:6px;overflow:auto;}}\n\
     code{{background:#f6f8fa;padding:.15em .35em;border-radius:4px;}}\n\
     pre code{{background:none;padding:0;}}\n\
     blockquote{{margin:0;padding-left:1rem;border-left:3px solid #d0d7de;color:#57606a;}}\n\
     table{{width:100%;border-collapse:collapse;}}\n\
     td,th{{border:1px solid #d0d7de;padding:.4em .6em;}}\n\
     hr{{border:none;border-top:1px solid #d0d7de;margin:2rem 0;}}\n\
     h1{{margin-bottom:1.5rem;}}\n\
     .mica-footer{{margin-top:3rem;padding-top:1rem;border-top:1px solid #eaeef2;\
     color:#8c959f;font-size:.85rem;}}\n</style>\n</head>\n<body>\n\
     <h1>{safe_title}</h1>\n{body_html}\n\
     <div class=\"mica-footer\">用 Mica 制作</div>\n</body>\n</html>\n"
  )
}

/// Minimal HTML-text escaping for the title (the body is escaped by export_html).
fn escape_html_min(s: &str) -> String {
  s.replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
}
