use axum::{
  Router,
  routing::{delete, get, patch, post},
};
use mica_app_core::AppState;

mod ai;
mod ai_ws;
pub mod auth;
mod avatar;
mod comments;
mod documents;
mod files;
mod health;
mod legal;
mod history;
mod import;
mod email_verify;
mod password_reset;
mod tokens;
mod workspaces;
pub mod ws;

/// Top-level WebSocket routes, mounted outside the `/api` prefix.
pub fn ws_router() -> Router<AppState> {
  Router::new()
    .route(
      "/ws/workspaces/{workspace_id}/documents/{document_id}",
      get(ws::document_socket),
    )
    .route("/ws/ai", get(ai_ws::ai_socket))
}

/// The public share page, mounted OUTSIDE `/api` so it never sees the auth
/// scope-guard — the token in the path is the only credential. Clean URL
/// `/s/{token}`; nginx proxies `/s/` to the backend.
/// The privacy statement and terms of service. Outside `/api` and unauthenticated
/// — a user has to be able to read what an instance does with their data BEFORE
/// they have an account, and to open the link someone sent them. Needs an nginx
/// rule like the pages below (see `legal.rs`).
pub fn legal_router() -> Router<AppState> {
  legal::router()
}

pub fn share_router() -> Router<AppState> {
  Router::new().route("/s/{token}", get(documents::public_share_page))
}

/// The server-rendered inbox-proof pages, mounted OUTSIDE `/api` like the share
/// page: `GET/POST /reset-password` and `GET /verify-email`. The token in the URL
/// or form is the only credential, so neither may sit behind the auth scope-guard.
///
/// **Both need an nginx rule.** They are exact-match `location`s proxied to the
/// backend ahead of the SPA fallback (deploy/nginx.conf, nginx.dev.conf); without
/// one, the link lands on index.html and the user sees the app instead of the
/// page. Adding a third page here means adding it there too.
pub fn reset_router() -> Router<AppState> {
  password_reset::router().merge(email_verify::router())
}

pub fn api_router() -> Router<AppState> {
  Router::new()
    .route("/health", get(health::health))
    .route("/ready", get(health::ready))
    .route("/auth/register", post(auth::register))
    .route(
      "/auth/resend-verification",
      post(email_verify::resend),
    )
    .route("/auth/login", post(auth::login))
    .route("/auth/refresh", post(auth::refresh))
    .route("/auth/logout", post(auth::logout))
    .route(
      "/auth/me",
      get(auth::me).patch(auth::update_me).delete(auth::delete_account),
    )
    .route(
      "/auth/me/avatar",
      // 4 MB cap enforced in the handler; the layer only has to stop axum's 2 MB
      // default from rejecting a legitimate photo before the handler can say so.
      axum::routing::put(avatar::put_avatar)
        .delete(avatar::delete_avatar)
        .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024)),
    )
    // Public by design (see avatar.rs): an <img> cannot carry a bearer token.
    .route("/users/{user_id}/avatar", get(avatar::get_avatar))
    .route("/auth/password", post(auth::change_password))
    .route(
      "/auth/password/forgot",
      post(password_reset::forgot),
    )
    .route(
      "/auth/tokens",
      get(tokens::list_tokens).post(tokens::create_token),
    )
    .route("/auth/tokens/{id}", delete(tokens::revoke_token))
    .route("/export/markdown", get(documents::export_all_markdown))
    .route(
      "/workspaces/{workspace_id}/views/{view_id}/export.zip",
      get(documents::export_folder_zip),
    )
    .route(
      "/workspaces/export.zip",
      get(documents::export_all_workspaces_zip),
    )
    .route(
      "/workspaces/export/stats",
      get(documents::export_all_stats),
    )
    .route(
      "/workspaces/{workspace_id}/export.zip",
      get(documents::export_workspace_zip),
    )
    .route(
      "/workspaces/{workspace_id}/export/markdown",
      get(documents::export_workspace_markdown),
    )
    .route("/ai/complete", post(ai::complete))
    .route(
      "/ai/settings",
      get(ai::get_settings).patch(ai::update_settings),
    )
    .route(
      "/workspaces",
      get(workspaces::list).post(workspaces::create),
    )
    .route("/workspaces/reorder", post(workspaces::reorder))
    .route(
      "/workspaces/{workspace_id}",
      get(workspaces::get)
        .patch(workspaces::update)
        .delete(workspaces::delete),
    )
    .route(
      "/workspaces/{workspace_id}/usage",
      get(workspaces::workspace_usage),
    )
    .route(
      "/workspaces/{workspace_id}/members",
      get(workspaces::list_members).post(workspaces::add_member),
    )
    .route(
      "/workspaces/{workspace_id}/members/{user_id}",
      patch(workspaces::update_member).delete(workspaces::remove_member),
    )
    .route(
      "/workspaces/{workspace_id}/views",
      get(documents::list_views),
    )
    .route(
      "/workspaces/{workspace_id}/search",
      get(documents::search_workspace),
    )
    .route(
      "/workspaces/{workspace_id}/views/{view_id}",
      patch(documents::update_view).delete(documents::delete_view),
    )
    .route(
      "/workspaces/{workspace_id}/views/{view_id}/backlinks",
      get(documents::backlinks),
    )
    .route(
      "/workspaces/{workspace_id}/views/{view_id}/move",
      post(documents::move_view),
    )
    .route(
      "/workspaces/{workspace_id}/views/{view_id}/transfer",
      post(documents::transfer_view),
    )
    .route(
      "/workspaces/{workspace_id}/views/{view_id}/clone",
      post(documents::clone_view),
    )
    .route(
      "/workspaces/{workspace_id}/views/reorder",
      post(documents::reorder_views),
    )
    .route(
      "/workspaces/{workspace_id}/views/{view_id}/restore",
      post(documents::restore_view),
    )
    .route(
      "/workspaces/{workspace_id}/trash",
      // DELETE on the collection empties the bin; DELETE on a member (next
      // route) purges one subtree.
      get(documents::list_trash).delete(documents::purge_workspace_trash),
    )
    .route(
      "/workspaces/{workspace_id}/trash/{view_id}",
      delete(documents::purge_view),
    )
    .route(
      "/workspaces/import",
      post(import::start_import).layer(axum::extract::DefaultBodyLimit::max(1024 * 1024 * 1024)),
    )
    .route("/import/jobs/{job_id}", get(import::import_job))
    .route(
      "/import/jobs/{job_id}/cancel",
      post(import::cancel_import_job),
    )
    .route(
      "/workspaces/{workspace_id}/documents",
      post(documents::create_document),
    )
    .route(
      "/workspaces/{workspace_id}/folders",
      post(documents::create_folder),
    )
    .route(
      "/workspaces/{workspace_id}/documents/import/markdown",
      post(documents::import_document_markdown),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/bootstrap",
      get(documents::bootstrap_document),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/updates",
      post(documents::apply_document_update),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/export/markdown",
      get(documents::export_document_markdown),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/outline",
      get(documents::document_outline),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/markdown",
      patch(documents::update_document_markdown),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/rehost-image",
      // Body is raw image bytes (≤ the 25 MB upload cap); lift the default 2 MB.
      post(documents::rehost_image).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)),
    )
    // Comments (docs/comments-plan.md). Anchored to the text by yrs sticky index
    // and stored BESIDE the document, so its Markdown — and the round-trip
    // invariant — stay untouched. Writes need the `commenter` role.
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/comments",
      get(comments::list).post(comments::create),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/comments/{thread_id}",
      axum::routing::delete(comments::delete),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/comments/{thread_id}/reply",
      post(comments::reply),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/comments/{thread_id}/resolve",
      post(comments::resolve),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/export.zip",
      get(documents::export_document_zip),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/export/html",
      get(documents::export_document_html),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/share",
      get(documents::get_share)
        .post(documents::create_share)
        .delete(documents::delete_share),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/history",
      get(history::get_history),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/versions",
      post(history::create_version),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/versions/{version_id}",
      get(history::get_version),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/restore",
      post(history::restore),
    )
    .route(
      "/workspaces/{workspace_id}/files/presign",
      post(files::presign),
    )
    .route(
      "/workspaces/{workspace_id}/files/complete",
      post(files::complete),
    )
    .route(
      "/workspaces/{workspace_id}/files/resolve",
      post(files::resolve),
    )
    .route(
      "/workspaces/{workspace_id}/files/import-url",
      post(files::import_url),
    )
    .route(
      "/workspaces/{workspace_id}/files/{file_id}/blob",
      get(files::blob),
    )
    // Same link with a cosmetic filename so it reads (and saves) as an image;
    // the name is ignored — see files::blob.
    .route(
      "/workspaces/{workspace_id}/files/{file_id}/blob/{filename}",
      get(files::blob_named),
    )
    .route(
      "/workspaces/{workspace_id}/files/{file_id}",
      get(files::get_file).delete(files::delete_file),
    )
}
