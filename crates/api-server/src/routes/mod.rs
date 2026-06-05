use axum::{
  Router,
  routing::{delete, get, patch, post},
};
use mica_app_core::AppState;

mod ai;
mod ai_ws;
pub mod auth;
mod documents;
mod files;
mod health;
mod history;
mod import;
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

pub fn api_router() -> Router<AppState> {
  Router::new()
    .route("/health", get(health::health))
    .route("/ready", get(health::ready))
    .route("/auth/register", post(auth::register))
    .route("/auth/login", post(auth::login))
    .route("/auth/me", get(auth::me).patch(auth::update_me))
    .route("/auth/password", post(auth::change_password))
    .route("/export/markdown", get(documents::export_all_markdown))
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
    .route(
      "/workspaces/{workspace_id}",
      get(workspaces::get)
        .patch(workspaces::update)
        .delete(workspaces::delete),
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
      "/workspaces/{workspace_id}/views/{view_id}/move",
      post(documents::move_view),
    )
    .route(
      "/workspaces/{workspace_id}/views/{view_id}/restore",
      post(documents::restore_view),
    )
    .route(
      "/workspaces/{workspace_id}/trash",
      get(documents::list_trash),
    )
    .route(
      "/workspaces/{workspace_id}/trash/{view_id}",
      delete(documents::purge_view),
    )
    .route(
      "/workspaces/import",
      post(import::start_import)
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024 * 1024)),
    )
    .route("/import/jobs/{job_id}", get(import::import_job))
    .route(
      "/workspaces/{workspace_id}/documents",
      post(documents::create_document),
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
      "/workspaces/{workspace_id}/documents/{document_id}/export.zip",
      get(documents::export_document_zip),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/export/html",
      get(documents::export_document_html),
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
    .route(
      "/workspaces/{workspace_id}/files/{file_id}",
      get(files::get_file).delete(files::delete_file),
    )
}
