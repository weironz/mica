//! Workspace invitations — "add a member" is an offer the invitee accepts.
//!
//! `POST /workspaces/{id}/members` used to INSERT the membership directly: the
//! other person never agreed to join, and an unregistered address answered 404,
//! which let any admin ask "is this email registered?". Now it records an
//! invitation keyed by the normalized email and answers the same way whether an
//! account exists or not; the invitee sees it under `GET /invitations` (once
//! they hold a VERIFIED account with that address) and accepts or declines.
//!
//! The handlers are thin; the rules live in the pool-taking functions below so
//! the Postgres tests exercise exactly what production runs.

use axum::{
  Json,
  extract::{Path, State},
  http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use mica_app_core::AppState;
use mica_infra::{ApiError, ApiResult};
use serde::Serialize;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::auth::{existing_user_id_from_headers, normalize_email, user_id_from_headers};
use super::workspaces::{
  Workspace, ensure_can_manage_members, fetch_workspace_for_user, next_member_position,
  normalize_member_role,
};

/// What an admin sees: the pending invitation itself. Deliberately carries no
/// user id — whether the email belongs to an account is exactly what the
/// response must not say.
#[derive(Debug, Serialize, FromRow)]
pub struct Invitation {
  id: Uuid,
  workspace_id: Uuid,
  email: String,
  role: String,
  invited_by: Uuid,
  created_at: DateTime<Utc>,
}

/// What the invitee sees: enough to decide (which workspace, who asked, as what).
#[derive(Debug, Serialize, FromRow)]
pub struct ReceivedInvitation {
  id: Uuid,
  workspace_id: Uuid,
  workspace_name: String,
  role: String,
  invited_by_name: String,
  created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct InvitationResponse {
  invitation: Invitation,
}

#[derive(Debug, Serialize)]
pub struct InvitationListResponse {
  invitations: Vec<Invitation>,
}

#[derive(Debug, Serialize)]
pub struct ReceivedInvitationListResponse {
  invitations: Vec<ReceivedInvitation>,
}

#[derive(Debug, Serialize)]
pub struct AcceptResponse {
  workspace: Workspace,
}

// Every invitee-side query reads the caller's address only for a LIVE account
// whose address was CONFIRMED (`deleted_at IS NULL AND email_verified_at IS NOT
// NULL`). An unverified address proves nothing about who holds the inbox, so it
// must not unlock invitations sent to it; a tombstone (C4) has neither.

// ---- admin side --------------------------------------------------------------

/// Create (or refresh the role of) an invitation. Same answer for a registered
/// and an unregistered address — no user lookup happens at all. An address that
/// is ALREADY a member is a 409: the admin can see the member list anyway, so
/// saying so leaks nothing, and silently "succeeding" would leave a dead
/// invitation behind that accepting could never act on.
pub(crate) async fn create_invitation(
  db: &PgPool,
  workspace_id: Uuid,
  actor_id: Uuid,
  email: &str,
  role: &str,
) -> ApiResult<Invitation> {
  ensure_can_manage_members(db, workspace_id, actor_id).await?;
  let email = normalize_email(email)?;
  let role = normalize_member_role(role)?;

  let already_member: bool = sqlx::query_scalar(
    r#"
      SELECT EXISTS (
        SELECT 1 FROM workspace_members wm
        JOIN users u ON u.id = wm.user_id
        WHERE wm.workspace_id = $1 AND u.email = $2
      )
    "#,
  )
  .bind(workspace_id)
  .bind(&email)
  .fetch_one(db)
  .await?;
  if already_member {
    return Err(ApiError::Conflict(
      "already a member of this workspace".to_string(),
    ));
  }

  let invitation = sqlx::query_as::<_, Invitation>(
    r#"
      INSERT INTO workspace_invitations (workspace_id, email, role, invited_by)
      VALUES ($1, $2, $3::workspace_role, $4)
      ON CONFLICT (workspace_id, email) DO UPDATE
      SET role = EXCLUDED.role, invited_by = EXCLUDED.invited_by
      RETURNING id, workspace_id, email, role::text AS role, invited_by, created_at
    "#,
  )
  .bind(workspace_id)
  .bind(&email)
  .bind(&role)
  .bind(actor_id)
  .fetch_one(db)
  .await?;
  Ok(invitation)
}

pub(crate) async fn list_workspace_invitations(
  db: &PgPool,
  workspace_id: Uuid,
  actor_id: Uuid,
) -> ApiResult<Vec<Invitation>> {
  ensure_can_manage_members(db, workspace_id, actor_id).await?;
  let rows = sqlx::query_as::<_, Invitation>(
    "SELECT id, workspace_id, email, role::text AS role, invited_by, created_at
     FROM workspace_invitations WHERE workspace_id = $1 ORDER BY created_at, email",
  )
  .bind(workspace_id)
  .fetch_all(db)
  .await?;
  Ok(rows)
}

pub(crate) async fn revoke_invitation(
  db: &PgPool,
  workspace_id: Uuid,
  actor_id: Uuid,
  invitation_id: Uuid,
) -> ApiResult<()> {
  ensure_can_manage_members(db, workspace_id, actor_id).await?;
  let deleted =
    sqlx::query("DELETE FROM workspace_invitations WHERE id = $1 AND workspace_id = $2")
      .bind(invitation_id)
      .bind(workspace_id)
      .execute(db)
      .await?
      .rows_affected();
  if deleted == 0 {
    return Err(ApiError::NotFound);
  }
  Ok(())
}

// ---- invitee side ------------------------------------------------------------

pub(crate) async fn received_invitations(
  db: &PgPool,
  user_id: Uuid,
) -> ApiResult<Vec<ReceivedInvitation>> {
  let rows = sqlx::query_as::<_, ReceivedInvitation>(
    r#"
      SELECT i.id, i.workspace_id, w.name AS workspace_name, i.role::text AS role,
             inviter.display_name AS invited_by_name, i.created_at
      FROM users me
      JOIN workspace_invitations i ON i.email = me.email
      JOIN workspaces w ON w.id = i.workspace_id
      JOIN users inviter ON inviter.id = i.invited_by
      WHERE me.id = $1 AND me.deleted_at IS NULL AND me.email_verified_at IS NOT NULL
      ORDER BY i.created_at, i.id
    "#,
  )
  .bind(user_id)
  .fetch_all(db)
  .await?;
  Ok(rows)
}

/// Accept: consume the invitation and join, in one transaction. Returns the
/// workspace id. Someone else's invitation (or a consumed one) is a 404 — not a
/// 403, which would confirm the id exists. Joining is `ON CONFLICT DO NOTHING`:
/// accepting while already a member never rewrites a role (the 0028 lesson — an
/// upsert here would let a stale invitation demote someone).
pub(crate) async fn accept_invitation(
  db: &PgPool,
  user_id: Uuid,
  invitation_id: Uuid,
) -> ApiResult<Uuid> {
  let mut tx = db.begin().await?;
  // `FOR SHARE`: a concurrent account deletion locks this row `FOR UPDATE`, so
  // it cannot interleave with the membership insert below.
  let email: String = sqlx::query_scalar(
    "SELECT email FROM users
     WHERE id = $1 AND deleted_at IS NULL AND email_verified_at IS NOT NULL
     FOR SHARE",
  )
  .bind(user_id)
  .fetch_optional(&mut *tx)
  .await?
  .ok_or(ApiError::NotFound)?;
  // DELETE … RETURNING is the claim: of two concurrent accepts only one gets
  // the row, the other sees nothing and answers 404.
  let (workspace_id, role): (Uuid, String) = sqlx::query_as(
    "DELETE FROM workspace_invitations WHERE id = $1 AND email = $2
     RETURNING workspace_id, role::text",
  )
  .bind(invitation_id)
  .bind(&email)
  .fetch_optional(&mut *tx)
  .await?
  .ok_or(ApiError::NotFound)?;
  // Reads committed memberships only; the new row is not in yet.
  let position = next_member_position(db, user_id).await?;
  sqlx::query(
    "INSERT INTO workspace_members (workspace_id, user_id, role, position)
     VALUES ($1, $2, $3::workspace_role, $4)
     ON CONFLICT (workspace_id, user_id) DO NOTHING",
  )
  .bind(workspace_id)
  .bind(user_id)
  .bind(&role)
  .bind(position)
  .execute(&mut *tx)
  .await?;
  tx.commit().await?;
  Ok(workspace_id)
}

pub(crate) async fn decline_invitation(
  db: &PgPool,
  user_id: Uuid,
  invitation_id: Uuid,
) -> ApiResult<()> {
  let deleted = sqlx::query(
    "DELETE FROM workspace_invitations WHERE id = $1 AND email = (
       SELECT email FROM users
       WHERE id = $2 AND deleted_at IS NULL AND email_verified_at IS NOT NULL
     )",
  )
  .bind(invitation_id)
  .bind(user_id)
  .execute(db)
  .await?
  .rows_affected();
  if deleted == 0 {
    return Err(ApiError::NotFound);
  }
  Ok(())
}

// ---- handlers ----------------------------------------------------------------

/// `POST /workspaces/{workspace_id}/members` — kept at its old URL so existing
/// clients keep working; the body is now `{ "invitation": … }`.
pub async fn invite_member(
  state: &AppState,
  headers: &HeaderMap,
  workspace_id: Uuid,
  email: &str,
  role: &str,
) -> ApiResult<Json<InvitationResponse>> {
  let actor_id = user_id_from_headers(state, headers).await?;
  let invitation = create_invitation(&state.db, workspace_id, actor_id, email, role).await?;
  Ok(Json(InvitationResponse { invitation }))
}

/// `GET /workspaces/{workspace_id}/invitations`
pub async fn list_for_workspace(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(workspace_id): Path<Uuid>,
) -> ApiResult<Json<InvitationListResponse>> {
  let actor_id = user_id_from_headers(&state, &headers).await?;
  let invitations = list_workspace_invitations(&state.db, workspace_id, actor_id).await?;
  Ok(Json(InvitationListResponse { invitations }))
}

/// `DELETE /workspaces/{workspace_id}/invitations/{invitation_id}`
pub async fn revoke(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path((workspace_id, invitation_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
  let actor_id = user_id_from_headers(&state, &headers).await?;
  revoke_invitation(&state.db, workspace_id, actor_id, invitation_id).await?;
  Ok(StatusCode::NO_CONTENT)
}

/// `GET /invitations`
pub async fn list_received(
  State(state): State<AppState>,
  headers: HeaderMap,
) -> ApiResult<Json<ReceivedInvitationListResponse>> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  let invitations = received_invitations(&state.db, user_id).await?;
  Ok(Json(ReceivedInvitationListResponse { invitations }))
}

/// `POST /invitations/{invitation_id}/accept`
pub async fn accept(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(invitation_id): Path<Uuid>,
) -> ApiResult<Json<AcceptResponse>> {
  // A deleted account's JWT outlives it (see `existing_user_id_from_headers`);
  // joining a workspace is exactly what it must no longer be able to do.
  let user_id = existing_user_id_from_headers(&state, &headers).await?;
  let workspace_id = accept_invitation(&state.db, user_id, invitation_id).await?;
  let workspace = fetch_workspace_for_user(&state.db, workspace_id, user_id)
    .await?
    .ok_or(ApiError::NotFound)?;
  Ok(Json(AcceptResponse { workspace }))
}

/// `POST /invitations/{invitation_id}/decline`
pub async fn decline(
  State(state): State<AppState>,
  headers: HeaderMap,
  Path(invitation_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
  let user_id = user_id_from_headers(&state, &headers).await?;
  decline_invitation(&state.db, user_id, invitation_id).await?;
  Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod invitations_pg {
  use super::*;

  async fn pool() -> Option<PgPool> {
    let url = mica_infra::test_database_url()?;
    Some(
      PgPool::connect(&url)
        .await
        .expect("DATABASE_URL is set but the connection failed"),
    )
  }

  /// A verified, live account with a unique address.
  async fn user(db: &PgPool) -> (Uuid, String) {
    let id = Uuid::new_v4();
    // Not derived from `id`: one test asserts the id never appears in a response.
    let email = format!("{}@invite.test", Uuid::new_v4());
    sqlx::query(
      "INSERT INTO users(id,email,display_name,password_hash,email_verified_at)
       VALUES($1,$2,'Inviter','x',now())",
    )
    .bind(id)
    .bind(&email)
    .execute(db)
    .await
    .unwrap();
    (id, email)
  }

  /// owner → workspace (owner membership), returns workspace id.
  async fn workspace(db: &PgPool, owner: Uuid) -> Uuid {
    let ws = Uuid::new_v4();
    sqlx::query("INSERT INTO workspaces(id,name,owner_id) VALUES($1,'Team W',$2)")
      .bind(ws)
      .bind(owner)
      .execute(db)
      .await
      .unwrap();
    sqlx::query(
      "INSERT INTO workspace_members(workspace_id,user_id,role,position) VALUES($1,$2,'owner','0000000010')",
    )
    .bind(ws)
    .bind(owner)
    .execute(db)
    .await
    .unwrap();
    ws
  }

  async fn role_of(db: &PgPool, ws: Uuid, user: Uuid) -> Option<String> {
    sqlx::query_scalar(
      "SELECT role::text FROM workspace_members WHERE workspace_id=$1 AND user_id=$2",
    )
    .bind(ws)
    .bind(user)
    .fetch_optional(db)
    .await
    .unwrap()
  }

  async fn cleanup(db: &PgPool, workspaces: &[Uuid], users: &[Uuid]) {
    for ws in workspaces {
      sqlx::query("DELETE FROM workspaces WHERE id=$1")
        .bind(ws)
        .execute(db)
        .await
        .ok();
    }
    for u in users {
      sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(u)
        .execute(db)
        .await
        .ok();
    }
  }

  /// THE CHANGE: inviting no longer makes anyone a member; accepting does,
  /// with the invited role, and consumes the invitation. A second accept finds
  /// nothing to consume (404) and leaves the membership as it is.
  #[tokio::test]
  async fn inviting_does_not_add_a_member_until_the_invitee_accepts() {
    let Some(db) = pool().await else { return };
    let (owner, _) = user(&db).await;
    let (invitee, invitee_email) = user(&db).await;
    let ws = workspace(&db, owner).await;

    // Mixed case and padding: normalized like users.email.
    let inv = create_invitation(
      &db,
      ws,
      owner,
      &format!("  {}  ", invitee_email.to_uppercase()),
      "editor",
    )
    .await
    .unwrap();
    assert_eq!(inv.email, invitee_email);
    assert_eq!(
      role_of(&db, ws, invitee).await,
      None,
      "an invitation is not a membership"
    );

    let received = received_invitations(&db, invitee).await.unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].workspace_name, "Team W");
    assert_eq!(received[0].invited_by_name, "Inviter");
    assert_eq!(received[0].role, "editor");

    assert_eq!(accept_invitation(&db, invitee, inv.id).await.unwrap(), ws);
    assert_eq!(role_of(&db, ws, invitee).await.as_deref(), Some("editor"));
    assert!(received_invitations(&db, invitee).await.unwrap().is_empty());
    assert!(
      list_workspace_invitations(&db, ws, owner)
        .await
        .unwrap()
        .is_empty()
    );

    // Repeat accept: nothing left to consume, membership untouched.
    assert!(matches!(
      accept_invitation(&db, invitee, inv.id).await,
      Err(ApiError::NotFound)
    ));
    assert_eq!(role_of(&db, ws, invitee).await.as_deref(), Some("editor"));

    // Inviting an existing member is a 409, not a silent role rewrite.
    assert!(matches!(
      create_invitation(&db, ws, owner, &invitee_email, "viewer").await,
      Err(ApiError::Conflict(_))
    ));
    // …and the owner can never be invited (already a member) or invited AS owner.
    let owner_email: String = sqlx::query_scalar("SELECT email FROM users WHERE id=$1")
      .bind(owner)
      .fetch_one(&db)
      .await
      .unwrap();
    assert!(matches!(
      create_invitation(&db, ws, owner, &owner_email, "viewer").await,
      Err(ApiError::Conflict(_))
    ));
    assert!(matches!(
      create_invitation(&db, ws, owner, "someone@else.test", "owner").await,
      Err(ApiError::BadRequest(_))
    ));

    // Removed members can be invited again.
    sqlx::query("DELETE FROM workspace_members WHERE workspace_id=$1 AND user_id=$2")
      .bind(ws)
      .bind(invitee)
      .execute(&db)
      .await
      .unwrap();
    let again = create_invitation(&db, ws, owner, &invitee_email, "viewer")
      .await
      .unwrap();
    accept_invitation(&db, invitee, again.id).await.unwrap();
    assert_eq!(role_of(&db, ws, invitee).await.as_deref(), Some("viewer"));

    cleanup(&db, &[ws], &[owner, invitee]).await;
  }

  #[tokio::test]
  async fn declining_deletes_the_invitation_and_joins_nothing() {
    let Some(db) = pool().await else { return };
    let (owner, _) = user(&db).await;
    let (invitee, invitee_email) = user(&db).await;
    let ws = workspace(&db, owner).await;
    let inv = create_invitation(&db, ws, owner, &invitee_email, "viewer")
      .await
      .unwrap();

    decline_invitation(&db, invitee, inv.id).await.unwrap();
    assert_eq!(role_of(&db, ws, invitee).await, None);
    assert!(received_invitations(&db, invitee).await.unwrap().is_empty());
    assert!(matches!(
      decline_invitation(&db, invitee, inv.id).await,
      Err(ApiError::NotFound)
    ));
    assert!(matches!(
      accept_invitation(&db, invitee, inv.id).await,
      Err(ApiError::NotFound)
    ));

    cleanup(&db, &[ws], &[owner, invitee]).await;
  }

  /// Only the holder of the invited address can see, accept or decline it —
  /// and only once that address is VERIFIED. A stranger gets 404 (not 403,
  /// which would confirm the id exists). A tombstoned account has neither.
  #[tokio::test]
  async fn only_the_verified_owner_of_the_address_can_act_on_it() {
    let Some(db) = pool().await else { return };
    let (owner, _) = user(&db).await;
    let (invitee, invitee_email) = user(&db).await;
    let (stranger, _) = user(&db).await;
    let ws = workspace(&db, owner).await;
    let inv = create_invitation(&db, ws, owner, &invitee_email, "editor")
      .await
      .unwrap();

    assert!(
      received_invitations(&db, stranger)
        .await
        .unwrap()
        .is_empty()
    );
    assert!(matches!(
      accept_invitation(&db, stranger, inv.id).await,
      Err(ApiError::NotFound)
    ));
    assert!(matches!(
      decline_invitation(&db, stranger, inv.id).await,
      Err(ApiError::NotFound)
    ));
    assert_eq!(role_of(&db, ws, stranger).await, None);

    // Unverified address: the invitation stays invisible and unusable.
    sqlx::query("UPDATE users SET email_verified_at = NULL WHERE id=$1")
      .bind(invitee)
      .execute(&db)
      .await
      .unwrap();
    assert!(received_invitations(&db, invitee).await.unwrap().is_empty());
    assert!(matches!(
      accept_invitation(&db, invitee, inv.id).await,
      Err(ApiError::NotFound)
    ));

    // Tombstone (C4), even if a row somehow kept the address and a verified mark.
    sqlx::query("UPDATE users SET email_verified_at = now(), deleted_at = now() WHERE id=$1")
      .bind(invitee)
      .execute(&db)
      .await
      .unwrap();
    assert!(received_invitations(&db, invitee).await.unwrap().is_empty());
    assert!(matches!(
      accept_invitation(&db, invitee, inv.id).await,
      Err(ApiError::NotFound)
    ));
    assert_eq!(role_of(&db, ws, invitee).await, None);

    // Non-admins cannot invite, list or revoke.
    sqlx::query("INSERT INTO workspace_members(workspace_id,user_id,role,position) VALUES($1,$2,'editor','0000000010')")
      .bind(ws)
      .bind(stranger)
      .execute(&db)
      .await
      .unwrap();
    assert!(matches!(
      create_invitation(&db, ws, stranger, "x@y.test", "viewer").await,
      Err(ApiError::Forbidden)
    ));
    assert!(matches!(
      list_workspace_invitations(&db, ws, stranger).await,
      Err(ApiError::Forbidden)
    ));
    assert!(matches!(
      revoke_invitation(&db, ws, stranger, inv.id).await,
      Err(ApiError::Forbidden)
    ));

    cleanup(&db, &[ws], &[owner, invitee, stranger]).await;
  }

  /// THE ENUMERATION CHANNEL: an unregistered address used to answer 404. Now
  /// the two cases produce the same response shape (no user id anywhere), and
  /// the unregistered one becomes visible the moment that person registers.
  #[tokio::test]
  async fn registered_and_unregistered_addresses_answer_alike() {
    let Some(db) = pool().await else { return };
    let (owner, _) = user(&db).await;
    let (registered, registered_email) = user(&db).await;
    let ws = workspace(&db, owner).await;
    let nobody_email = format!("{}@nobody.test", Uuid::new_v4());

    let a = serde_json::to_value(InvitationResponse {
      invitation: create_invitation(&db, ws, owner, &registered_email, "viewer")
        .await
        .unwrap(),
    })
    .unwrap();
    let b = serde_json::to_value(InvitationResponse {
      invitation: create_invitation(&db, ws, owner, &nobody_email, "viewer")
        .await
        .unwrap(),
    })
    .unwrap();
    let keys = |v: &serde_json::Value| {
      let mut k: Vec<String> = v["invitation"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
      k.sort();
      k
    };
    assert_eq!(keys(&a), keys(&b));
    assert!(
      !a.to_string().contains(&registered.to_string()),
      "the invitee's user id must not leak"
    );
    assert_eq!(a["invitation"]["role"], b["invitation"]["role"]);

    // Re-inviting updates the role in place (one row per address).
    create_invitation(&db, ws, owner, &nobody_email, "editor")
      .await
      .unwrap();
    let pending = list_workspace_invitations(&db, ws, owner).await.unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(
      pending
        .iter()
        .find(|i| i.email == nobody_email)
        .unwrap()
        .role,
      "editor"
    );

    // The unregistered address registers later and finds the invitation.
    let late = Uuid::new_v4();
    sqlx::query(
      "INSERT INTO users(id,email,display_name,password_hash,email_verified_at) VALUES($1,$2,'L','x',now())",
    )
    .bind(late)
    .bind(&nobody_email)
    .execute(&db)
    .await
    .unwrap();
    assert_eq!(received_invitations(&db, late).await.unwrap().len(), 1);

    cleanup(&db, &[ws], &[owner, registered, late]).await;
  }

  #[tokio::test]
  async fn revoking_and_deleting_the_workspace_remove_invitations() {
    let Some(db) = pool().await else { return };
    let (owner, _) = user(&db).await;
    let (invitee, invitee_email) = user(&db).await;
    let ws = workspace(&db, owner).await;
    let other_ws = workspace(&db, owner).await;

    let inv = create_invitation(&db, ws, owner, &invitee_email, "viewer")
      .await
      .unwrap();
    // Revoking through the wrong workspace finds nothing.
    assert!(matches!(
      revoke_invitation(&db, other_ws, owner, inv.id).await,
      Err(ApiError::NotFound)
    ));
    revoke_invitation(&db, ws, owner, inv.id).await.unwrap();
    assert!(received_invitations(&db, invitee).await.unwrap().is_empty());
    assert!(matches!(
      accept_invitation(&db, invitee, inv.id).await,
      Err(ApiError::NotFound)
    ));

    create_invitation(&db, ws, owner, &invitee_email, "viewer")
      .await
      .unwrap();
    sqlx::query("DELETE FROM workspaces WHERE id=$1")
      .bind(ws)
      .execute(&db)
      .await
      .unwrap();
    let left: i64 =
      sqlx::query_scalar("SELECT count(*) FROM workspace_invitations WHERE workspace_id=$1")
        .bind(ws)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(left, 0, "deleting the workspace cascades its invitations");
    assert!(received_invitations(&db, invitee).await.unwrap().is_empty());

    // The DB refuses an owner invitation even if a future path skips the API.
    let raw = sqlx::query(
      "INSERT INTO workspace_invitations(workspace_id,email,role,invited_by) VALUES($1,'a@b.test','owner',$2)",
    )
    .bind(other_ws)
    .bind(owner)
    .execute(&db)
    .await;
    assert!(raw.is_err(), "role 'owner' must be rejected by the CHECK");

    cleanup(&db, &[ws, other_ws], &[owner, invitee]).await;
  }
}
