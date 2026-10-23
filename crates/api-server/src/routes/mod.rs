use axum::{
  Router,
  routing::{delete, get, patch, post},
};
use mica_app_core::AppState;

mod ai;
mod ai_edits;
mod ai_ws;
pub mod auth;
mod avatar;
mod comments;
mod documents;
mod email_verify;
mod files;
mod health;
mod history;
mod import;
mod invitations;
mod legal;
mod names;
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
    .route(
      // "The sidebar tree changed" pings — see ws::views_socket.
      "/ws/workspaces/{workspace_id}/views",
      get(ws::views_socket),
    )
    .route("/ws/ai", get(ai_ws::ai_socket))
}

/// The privacy statement and terms of service. Outside `/api` and unauthenticated
/// — a user has to be able to read what an instance does with their data BEFORE
/// they have an account, and to open the link someone sent them. Needs an nginx
/// rule like the pages below (see `legal.rs`).
pub fn legal_router() -> Router<AppState> {
  legal::router()
}

/// The public share page, mounted OUTSIDE `/api` so it never sees the auth
/// scope-guard — the token in the path is the only credential. Clean URL
/// `/s/{token}`; nginx proxies `/s/` to the backend.
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

/// The UNGUARDED half of `/api` — routes whose caller has no signed-in session
/// (health probes, sign-in, token refresh, the forgot/resend mail endpoints —
/// by definition whoever needs them cannot authenticate first) or whose
/// credential is the link itself (the avatar `<img>`, the blob capability
/// URL). `main` mounts this beside [`layered_api`]'s guarded routes; nothing
/// here ever sees `auth::scope_guard`.
///
/// The per-IP throttle / Argon2 gate are attached to the ROUTES themselves
/// (`rate_limit::ip_throttled` / `argon2_throttled`), the layering that
/// replaced the path-suffix tables these used to be matched by — the blob and
/// resend-verification endpoints were both silently misclassified under that
/// scheme.
pub fn public_api_router() -> Router<AppState> {
  use crate::rate_limit::{argon2_throttled, ip_throttled};
  Router::new()
    .route("/health", get(health::health))
    .route("/ready", get(health::ready))
    // Sign-up and sign-in hash a password, so they carry the Argon2 gate too.
    .route("/auth/register", argon2_throttled(post(auth::register)))
    .route("/auth/login", argon2_throttled(post(auth::login)))
    // You refresh precisely BECAUSE the access token is dead — demanding a live
    // one here would be a deadlock; the refresh token in the body is the
    // credential. Throttled, but no Argon2 gate: rotation is a DB token swap,
    // and holding a scarce permit for it would starve real logins.
    .route("/auth/refresh", ip_throttled(post(auth::refresh)))
    // Signing out must work with a dead access token — the normal case for
    // someone who left the app closed. The refresh token is the credential.
    .route("/auth/logout", post(auth::logout))
    // Each call mails a fresh link; the throttle is the mail-quota protection.
    .route(
      "/auth/resend-verification",
      ip_throttled(post(email_verify::resend)),
    )
    // The caller has no session (that is the whole point); the emailed
    // /reset-password page (outside /api) is where the reset itself happens.
    .route(
      "/auth/password/forgot",
      ip_throttled(post(password_reset::forgot)),
    )
    // Public by design (see avatar.rs): an <img> cannot carry a bearer token.
    .route("/users/{user_id}/avatar", get(avatar::get_avatar))
    // The image capability link — the unguessable file_id IS the credential,
    // public so a copied Markdown image still renders in Typora / a browser
    // (files::blob). The trailing filename is cosmetic and ignored.
    .route(
      "/workspaces/{workspace_id}/files/{file_id}/blob",
      get(files::blob),
    )
    .route(
      "/workspaces/{workspace_id}/files/{file_id}/blob/{filename}",
      get(files::blob_named),
    )
}

/// The signed-in routes that nonetheless VERIFY the current password with
/// Argon2 — a stolen token plus no throttle was a password oracle that could
/// pin every core. [`layered_api`] stacks `rate_limit::throttle_argon2`
/// OUTSIDE the `scope_guard` for this pair — a hammering caller is 429'd, not
/// 401'd — the same order the old router-wide limiter sat in.
pub fn api_argon2_router() -> Router<AppState> {
  Router::new()
    .route("/auth/password", post(auth::change_password))
    .route("/auth/me", delete(auth::delete_account))
}

/// `/api` with its guard stack on: the protected [`api_router`] under
/// `scope_guard`, the [`api_argon2_router`] pair under guard + throttle, and
/// [`public_api_router`] bare. `main` mounts this at `/api`; `router_tests`
/// below drive it end to end.
pub fn layered_api(state: &AppState) -> Router<AppState> {
  let guarded = api_router().layer(axum::middleware::from_fn_with_state(
    state.clone(),
    auth::scope_guard,
  ));
  let argon2 = api_argon2_router()
    .layer(axum::middleware::from_fn_with_state(
      state.clone(),
      auth::scope_guard,
    ))
    // Added last = runs first: throttle outside the guard (see above).
    .layer(axum::middleware::from_fn(
      crate::rate_limit::throttle_argon2,
    ));
  guarded
    .merge(argon2)
    .merge(public_api_router())
    // Registered AFTER the guards on purpose: a path that does not exist is a
    // 404 for everyone, and answering 401 first sends people hunting for a
    // credential problem they do not have.
    .fallback(|| async { mica_infra::ApiError::NotFound })
}

pub fn api_router() -> Router<AppState> {
  Router::new()
    .route("/auth/me", get(auth::me).patch(auth::update_me))
    .route(
      "/auth/me/settings",
      get(auth::get_settings).put(auth::put_settings),
    )
    .route(
      "/auth/me/avatar",
      // 4 MB cap enforced in the handler; the layer only has to stop axum's 2 MB
      // default from rejecting a legitimate photo before the handler can say so.
      axum::routing::put(avatar::put_avatar)
        .delete(avatar::delete_avatar)
        .layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024)),
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
    .route("/workspaces/export/stats", get(documents::export_all_stats))
    .route(
      "/workspaces/{workspace_id}/export.zip",
      get(documents::export_workspace_zip),
    )
    .route(
      "/workspaces/{workspace_id}/export-manifest",
      get(documents::export_manifest),
    )
    .route(
      "/workspaces/{workspace_id}/export-pages/{view_id}/export.zip",
      get(documents::export_page_zip),
    )
    .route(
      "/workspaces/{workspace_id}/export/markdown",
      get(documents::export_workspace_markdown),
    )
    .route("/ai/complete", post(ai::complete))
    .route("/ai/edits/preview", post(ai_edits::preview))
    .route(
      "/ai/settings",
      get(ai::get_settings).patch(ai::update_settings),
    )
    .route("/ai/models", get(ai::list_models))
    .route(
      "/ai/providers/{provider_id}",
      axum::routing::delete(ai::delete_provider),
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
    // Invitations (invitations.rs). All signed-in: nothing here is public.
    .route(
      "/workspaces/{workspace_id}/invitations",
      get(invitations::list_for_workspace),
    )
    .route(
      "/workspaces/{workspace_id}/invitations/{invitation_id}",
      delete(invitations::revoke),
    )
    .route("/invitations", get(invitations::list_received))
    .route(
      "/invitations/{invitation_id}/accept",
      post(invitations::accept),
    )
    .route(
      "/invitations/{invitation_id}/decline",
      post(invitations::decline),
    )
    .route(
      "/workspaces/{workspace_id}/views",
      get(documents::list_views),
    )
    .route(
      "/workspaces/{workspace_id}/search",
      get(documents::search_workspace),
    )
    // Cross-workspace search. NOT a flag on the route above: the path is what
    // states the scope, and widening it by query parameter would make
    // `/workspaces/{id}/search` return things from other workspaces.
    .route("/search", get(documents::search_all_workspaces))
    .route(
      "/workspaces/{workspace_id}/views/{view_id}",
      patch(documents::update_view).delete(documents::delete_view),
    )
    .route(
      "/workspaces/{workspace_id}/views/{view_id}/backlinks",
      get(documents::backlinks),
    )
    .route("/workspaces/{workspace_id}/graph", get(documents::graph))
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
    // Batch forms of trash / restore / move. The per-view routes above stay as
    // they are — the app drives one view at a time and reads the returned tree
    // — while a reorganisation touching hundreds of pages gets one request
    // instead of hundreds. Every one reports what it ACTUALLY touched.
    .route(
      "/workspaces/{workspace_id}/views/batch-trash",
      post(documents::batch_trash_views),
    )
    .route(
      "/workspaces/{workspace_id}/views/batch-restore",
      post(documents::batch_restore_views),
    )
    .route(
      "/workspaces/{workspace_id}/views/batch-move",
      post(documents::batch_move_views),
    )
    // POST, and under /views rather than /trash — matching its siblings above.
    // A body of ids is what makes it a batch, and DELETE-with-a-body is the
    // shape proxies and HTTP clients are least reliable about.
    .route(
      "/workspaces/{workspace_id}/views/batch-purge",
      post(documents::batch_purge_views),
    )
    // Sibling of `/{view_id}/transfer`, with the whole selection in ONE
    // transaction. Both are the same core; N sequential single transfers were
    // the alternative, and those leave a half-moved selection on any failure.
    .route(
      "/workspaces/{workspace_id}/views/batch-transfer",
      post(documents::batch_transfer_views),
    )
    .route(
      "/workspaces/{workspace_id}/documents/batch-read",
      post(documents::batch_read_documents),
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
    .route("/import/jobs", get(import::import_history))
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
      // GET is the same handler as `/export/markdown` above, under the name
      // people actually reach for. Reading a page was only ever spelled
      // "export", which nobody guesses and no spec announced — one report of
      // finding it ran through 404s and `strings` on the binary. The old path
      // stays valid; this is a second door, not a move.
      get(documents::export_document_markdown).patch(documents::update_document_markdown),
    )
    .route(
      "/workspaces/{workspace_id}/documents/{document_id}/rehost-image",
      // Body is raw image bytes (≤ the 25 MB upload cap); lift the default 2 MB.
      post(documents::rehost_image).layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024)),
    )
    .route(
      // The other end of the same job: the image is gone for good, so the block
      // goes too. No body — the caller names the block and the dead url it
      // checked, and the server refuses if the block has moved on.
      "/workspaces/{workspace_id}/documents/{document_id}/drop-image",
      post(documents::drop_image),
    )
    // Comments (docs/features/comments-plan.md). Anchored to the text by yrs sticky index
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
    // The blob capability link GETs live in `public_api_router` above.
    .route(
      "/workspaces/{workspace_id}/files/{file_id}",
      get(files::get_file).delete(files::delete_file),
    )
}

#[cfg(test)]
#[path = "contract_gold.rs"]
mod contract_gold;

#[cfg(test)]
mod router_tests {
  use super::*;

  /// Building the router is where axum rejects an ambiguous route table — by
  /// panicking, at startup, long after `cargo check` has said everything is
  /// fine. Worth one test because the batch routes deliberately put a literal
  /// segment where a parameter already lives (`/views/batch-trash` alongside
  /// `/views/{view_id}`): that resolves in favour of the literal, but it is
  /// exactly the shape that would blow up a deploy if it ever stopped doing so.
  /// The MERGE of the three api halves is what is built — `/auth/me` is split
  /// across them (GET/PATCH guarded, DELETE guard + Argon2 gate) — because a
  /// conflict between the halves would panic here, at merge time.
  #[test]
  fn the_route_table_is_unambiguous() {
    let _ = api_router()
      .merge(public_api_router())
      .merge(api_argon2_router());
    let _ = ws_router();
    let _ = share_router();
    let _ = legal_router();
    let _ = reset_router();
  }
}

/// The classifications the retired path-suffix tables (`is_public` in auth.rs,
/// `is_rate_limited` / `spends_argon2` in rate_limit.rs) used to encode,
/// asserted as REQUESTS through the assembled, layered routers that replaced
/// the tables: the assertion is "this route is mounted where it carries this
/// layer", which is the property the tables could never keep in sync.
///
/// The pool is LAZY and the guard bucket is tiny (2 tokens, no refill), so a
/// guarded route answers 401 (never 429), a public route reaches its handler
/// (400/404/500, never 401), and "throttled" shows up as 429 on the third
/// rapid call from one client — distinct `x-forwarded-for` values keep the
/// per-IP buckets independent across sub-cases.
#[cfg(test)]
mod route_layer_tests {
  use super::*;
  use crate::rate_limit;
  use axum::Extension;
  use axum::body::Body;
  use axum::extract::ConnectInfo;
  use axum::http::{Method, Request, StatusCode};
  use mica_infra::{AppConfig, Environment};
  use std::net::{IpAddr, Ipv4Addr, SocketAddr};
  use tower::ServiceExt;

  const PEER: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(172, 20, 0, 5)), 40000);
  const WS: &str = "/api/workspaces/5a398481-bcf5-4a5e-83f0-a891bf0bad7e";
  const FID: &str = "55b3e5ff-4117-4d65-9434-0b17922d8e87";

  fn test_state() -> AppState {
    let config = AppConfig {
      environment: Environment::Development,
      http_addr: PEER,
      database_url: "postgres://127.0.0.1:1/mica".to_string(),
      database_max_connections: 1,
      jwt_secret: "route-layer-test-secret".to_string(),
      ws_min_protocol: 1,
      access_token_ttl_seconds: 60,
      refresh_token_ttl_seconds: 60,
      cors_allowed_origins: Vec::new(),
      seed_test_user: None,
      registration_enabled: false,
      workspace_quota_bytes: 0,
      owner_quota_bytes: 0,
      search_index_mlock: false,
      sync_tuning: Default::default(),
      app_base_url: "http://localhost:8080".to_string(),
    };
    // Lazy: nothing dials out unless a handler runs a query, and every route
    // asserted here is refused (by its layer) or fails fast before that.
    let db = sqlx::PgPool::connect_lazy(&config.database_url).expect("a lazy pool never fails");
    AppState::new(config, db, crate::mail::build_mailer())
  }

  fn app() -> Router {
    let state = test_state();
    Router::new()
      .nest("/api", layered_api(&state))
      .merge(ws_router())
      .merge(reset_router())
      .layer(Extension(rate_limit::test_guard(64)))
      .with_state(state)
  }

  async fn send(app: &Router, method: Method, path: &str, xff: &str) -> StatusCode {
    let mut req = Request::builder()
      .method(method)
      .uri(path)
      .header("x-forwarded-for", xff)
      .body(Body::empty())
      .unwrap();
    req.extensions_mut().insert(ConnectInfo(PEER));
    app.clone().oneshot(req).await.unwrap().status()
  }

  /// The caller has no session on any of these — by design (and previously
  /// by `is_public` table entries that kept getting out of sync).
  #[tokio::test]
  async fn the_public_half_answers_without_a_token() {
    let app = app();
    assert_eq!(
      send(&app, Method::GET, "/api/health", "198.51.100.1").await,
      StatusCode::OK
    );
    // The avatar <img> and the blob capability links: 404 here (no storage in
    // this harness), what matters is NOT 401.
    for path in [
      format!("/api/users/{FID}/avatar"),
      format!("{WS}/files/{FID}/blob"),
      format!("{WS}/files/{FID}/blob/diagram.png"),
      format!("{WS}/files/{FID}/blob/%E5%9B%BE.png"),
    ] {
      let status = send(&app, Method::GET, &path, "198.51.100.1").await;
      assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "{path} must not need a session"
      );
    }
  }

  #[tokio::test]
  async fn public_routes_do_not_widen_to_neighbour_paths() {
    let app = app();
    // Everything else under /files and /users stays behind the guard.
    for (method, path) in [
      (Method::GET, format!("{WS}/files/{FID}")),
      (Method::POST, format!("{WS}/files/presign")),
      (Method::POST, format!("{WS}/files/complete")),
      (Method::POST, format!("{WS}/files/resolve")),
      (Method::POST, format!("{WS}/files/import-url")),
      (Method::PUT, "/api/auth/me/avatar".to_string()),
      (Method::GET, "/api/auth/tokens".to_string()),
      (Method::POST, "/api/workspaces".to_string()),
    ] {
      let status = send(&app, method, &path, "198.51.100.2").await;
      assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} must be guarded");
    }
    // …and the extra-deep / lookalike paths are plain 404s, exactly as the
    // fallback answers for everyone.
    for path in [
      format!("{WS}/files/{FID}/blob/a/b"),
      format!("{WS}/documents/{FID}/blob"),
      format!("/api/users/{FID}/avatar/raw"),
      format!("/api/users/{FID}"),
      "/api/documents/abc".to_string(),
    ] {
      let status = send(&app, Method::GET, &path, "198.51.100.2").await;
      assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "{path} should be a plain 404"
      );
    }
  }

  /// THE BUG the route-level layering exists to kill: suffix matching made
  /// `/{id}`-style route params a scope-guard bypass whenever the value
  /// happened to spell "health" (or "logout", or "blob").
  #[tokio::test]
  async fn a_route_param_named_health_does_not_bypass_the_guard() {
    let app = app();
    assert_eq!(
      send(
        &app,
        Method::DELETE,
        "/api/ai/providers/health",
        "198.51.100.3"
      )
      .await,
      StatusCode::UNAUTHORIZED,
      "the param VALUE must never opt the route out of the scope guard"
    );
  }

  /// Burst is 2 per client and never refills: the third rapid call from one
  /// address must 429, on exactly the routes the old table throttled.
  #[tokio::test]
  async fn the_credential_and_mail_endpoints_are_throttled() {
    let app = app();
    let cases = [
      (Method::POST, "/api/auth/login", "198.51.100.10"),
      (Method::POST, "/api/auth/register", "198.51.100.11"),
      (Method::POST, "/api/auth/refresh", "198.51.100.12"),
      (Method::POST, "/api/auth/password/forgot", "198.51.100.13"),
      (
        Method::POST,
        "/api/auth/resend-verification",
        "198.51.100.14",
      ),
    ];
    for (method, path, xff) in cases {
      for attempt in 1..=3 {
        let status = send(&app, method.clone(), path, xff).await;
        if attempt < 3 {
          assert_ne!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "{path} attempt {attempt}: the burst should still be open (and it must not 401 \
             — the caller has no session yet)"
          );
        } else {
          assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "{path} must be throttled on the 3rd call"
          );
        }
      }
    }
    // The reset PAGE (outside /api): throttled too, including its GET — the
    // old suffix rule matched any method on this path.
    for attempt in 1..=3 {
      let status = send(&app, Method::GET, "/reset-password", "198.51.100.15").await;
      assert_eq!(
        status == StatusCode::TOO_MANY_REQUESTS,
        attempt == 3,
        "/reset-password attempt {attempt}"
      );
    }
  }

  /// …while nothing else is: these must answer 401 (guarded) however often
  /// one address calls, and the WS route never answers 429 at all.
  #[tokio::test]
  async fn ordinary_routes_are_never_throttled() {
    let app = app();
    for _ in 1..=4 {
      assert_eq!(
        send(&app, Method::POST, "/api/workspaces", "198.51.100.20").await,
        StatusCode::UNAUTHORIZED
      );
      assert_ne!(
        send(
          &app,
          Method::GET,
          "/ws/workspaces/x/documents/y",
          "198.51.100.20"
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS
      );
    }
  }

  /// Only the hashing routes carry the Argon2 gate: with every permit held,
  /// login sheds 429 while refresh / forgot / resend sail through (their cost
  /// is a DB row or an email, so holding a scarce permit for them would only
  /// starve real logins).
  #[tokio::test]
  async fn only_login_and_register_hold_an_argon2_permit() {
    let state = test_state();
    let guard = rate_limit::test_guard(1);
    let _held = guard.argon2_hog();
    let app = Router::new()
      .nest("/api", layered_api(&state))
      .merge(reset_router())
      .layer(Extension(guard))
      .with_state(state);
    for (path, xff) in [
      ("/api/auth/login", "198.51.100.30"),
      ("/api/auth/register", "198.51.100.35"),
    ] {
      assert_eq!(
        send(&app, Method::POST, path, xff).await,
        StatusCode::TOO_MANY_REQUESTS,
        "{path}: every Argon2 permit held → shed"
      );
    }
    for (path, xff) in [
      ("/api/auth/refresh", "198.51.100.31"),
      ("/api/auth/password/forgot", "198.51.100.32"),
      ("/api/auth/resend-verification", "198.51.100.33"),
      ("/reset-password", "198.51.100.34"),
    ] {
      let status = send(&app, Method::POST, path, xff).await;
      assert_ne!(
        status,
        StatusCode::TOO_MANY_REQUESTS,
        "{path} holds no Argon2 permit"
      );
    }
  }

  /// Signed-in Argon2 routes: throttled OUTSIDE the guard, so a hammering
  /// caller is 429'd (not merely 401'd) — the order the router-wide limiter
  /// used to sit in.
  #[tokio::test]
  async fn the_signed_in_password_checks_are_gated_and_throttled() {
    let app = app();
    for (method, path, xff) in [
      (Method::POST, "/api/auth/password", "198.51.100.40"),
      (Method::DELETE, "/api/auth/me", "198.51.100.41"),
    ] {
      assert_eq!(
        send(&app, method.clone(), path, xff).await,
        StatusCode::UNAUTHORIZED
      );
      assert_eq!(
        send(&app, method.clone(), path, xff).await,
        StatusCode::UNAUTHORIZED
      );
      assert_eq!(
        send(&app, method, path, xff).await,
        StatusCode::TOO_MANY_REQUESTS,
        "{path}: third rapid call must 429 even with no credential"
      );
    }
  }

  #[tokio::test]
  async fn the_profile_read_and_update_stay_free() {
    let app = app();
    for method in [Method::GET, Method::PATCH] {
      for _ in 1..=4 {
        assert_eq!(
          send(&app, method.clone(), "/api/auth/me", "198.51.100.50").await,
          StatusCode::UNAUTHORIZED,
          "{method} /api/auth/me: guarded, but never throttled"
        );
      }
    }
  }
}
