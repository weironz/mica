# Sync, History, and API Draft

## API Style

Use REST for ordinary request/response workflows and WebSocket for document rooms.

REST:

- authentication
- workspace management
- page tree
- document open bootstrap
- history listing
- restore
- export
- file upload
- search

WebSocket:

- document operations
- text delta updates
- presence/awareness
- server broadcasts

## Authentication

Email/password → a **short-lived JWT access token** + a **long-lived refresh
token**. Two kinds of credential because they answer opposite needs:

| | access token | refresh token |
|---|---|---|
| form | JWT, stateless | opaque random, a row in `refresh_tokens` |
| lifetime | `ACCESS_TOKEN_TTL_SECONDS`, default **24h** | `REFRESH_TOKEN_TTL_SECONDS`, default **30d** |
| revocable | **no** — nothing is stored to revoke | yes |
| used for | every API/WS call | `POST /api/auth/refresh`, nothing else |

The access token can't be recalled once issued, which is exactly why it stays
short. The refresh token is the revocable half, and it is the only thing that
mints new access tokens.

**Rotation + reuse detection.** A refresh token is single-use: refreshing stamps
`used_at` on it and issues a new one in the same `family_id` (one sign-in). Spend
an already-spent token and the server cannot tell "the client raced itself" from
"someone stole this" — so it assumes the worse and **revokes the whole family**.
The user signs in again; a thief gets nothing. (RFC 6819 §5.2.2.3.)

Spending is a conditional `UPDATE ... WHERE used_at IS NULL`, not a read-then-write:
that is what makes two concurrent refreshes resolve to exactly one winner, decided
by Postgres. The **client** must therefore never run two refreshes at once either
(see `SessionRefresher`) or it would burn its own session.

Changing a password revokes **all** of that user's families — people change
passwords because they think someone else is in there, so the intruder's refresh
token has to die with it. Access tokens already out there still live out their
≤24h; that is the stateless-JWT trade.

`/api/auth/refresh` is **public** by necessity: you refresh precisely because your
access token is dead, and the request's own credential is the refresh token in its
body. (`is_public`, pinned by a test — the same router-wide guard once silently
401'd the blob endpoint for a month.)

Never logged, never in a URL. Only the SHA-256 of a refresh token is stored;
plaintext exists once, in the response. Unsalted SHA-256 is right here and wrong
for a password: this is 244 bits of CSPRNG randomness (two v4 UUIDs, `getrandom`),
so there is nothing to brute-force and no reason to pay argon2's cost per request.

Preferred browser auth later:

- HttpOnly secure session cookie
- CSRF protection for mutating REST calls
- WebSocket authenticates through cookie or short-lived socket token

## REST Endpoints

### Auth

```text
POST /api/auth/register
POST /api/auth/login
POST /api/auth/refresh   # public: {refresh_token} -> a new access+refresh pair
GET  /api/auth/me
PATCH /api/auth/me
POST /api/auth/password  # revokes every session of this user
```

There is no `/logout`: signing out is a client-side drop of the pair. A server
one would only matter for revoking OTHER devices, which nothing asks for yet;
`revoke_user_sessions` is the piece that would back it.

### Workspaces

```text
GET    /api/workspaces
POST   /api/workspaces
GET    /api/workspaces/{workspace_id}
PATCH  /api/workspaces/{workspace_id}
GET    /api/workspaces/{workspace_id}/members
POST   /api/workspaces/{workspace_id}/members
PATCH  /api/workspaces/{workspace_id}/members/{user_id}
DELETE /api/workspaces/{workspace_id}/members/{user_id}
```

### Views / Page Tree

```text
GET    /api/workspaces/{workspace_id}/views
POST   /api/workspaces/{workspace_id}/views
PATCH  /api/workspaces/{workspace_id}/views/{view_id}
DELETE /api/workspaces/{workspace_id}/views/{view_id}
POST   /api/workspaces/{workspace_id}/views/{view_id}/move
```

### Documents

```text
POST /api/workspaces/{workspace_id}/documents
GET  /api/workspaces/{workspace_id}/documents/{document_id}
GET  /api/workspaces/{workspace_id}/documents/{document_id}/bootstrap
```

`bootstrap` returns:

```json
{
  "document_id": "uuid",
  "version": 120,
  "snapshot": {},
  "recent_updates": [],
  "permissions": {
    "can_read": true,
    "can_write": true,
    "can_comment": true
  }
}
```

### History

```text
GET  /api/workspaces/{workspace_id}/documents/{document_id}/history
GET  /api/workspaces/{workspace_id}/documents/{document_id}/versions/{version_id}
POST /api/workspaces/{workspace_id}/documents/{document_id}/versions
POST /api/workspaces/{workspace_id}/documents/{document_id}/restore
```

### Markdown

```text
POST /api/workspaces/{workspace_id}/documents/import/markdown
GET  /api/workspaces/{workspace_id}/documents/{document_id}/export/markdown
GET  /api/workspaces/{workspace_id}/documents/{document_id}/export/html
```

### Files

```text
POST   /api/workspaces/{workspace_id}/files/presign
POST   /api/workspaces/{workspace_id}/files/complete
DELETE /api/workspaces/{workspace_id}/files/{file_id}
```

### Search

```text
GET /api/workspaces/{workspace_id}/search?q=...
```

## WebSocket Endpoint

```text
GET /ws/workspaces/{workspace_id}/documents/{document_id}
```

## WebSocket Message Envelope

Client to server:

```json
{
  "id": "client-message-id",
  "type": "document.update",
  "document_id": "uuid",
  "client_seq": 12,
  "base_server_seq": 120,
  "payload": {}
}
```

Server to client:

```json
{
  "id": "server-message-id",
  "type": "document.update.accepted",
  "document_id": "uuid",
  "server_seq": 121,
  "actor_id": "uuid",
  "payload": {}
}
```

Error:

```json
{
  "id": "client-message-id",
  "type": "error",
  "code": "permission_denied",
  "message": "You do not have permission to edit this document"
}
```

## Message Types

Client sends:

- `document.update`
- `document.text_delta`
- `presence.update`
- `history.create_version`
- `ping`

Server sends:

- `document.bootstrap`
- `document.update.accepted`
- `document.text_delta.accepted`
- `presence.state`
- `presence.update`
- `history.snapshot_created`
- `error`
- `pong`

## Document Update Payload

```json
{
  "operations": [
    {
      "type": "insert_block",
      "block": {
        "id": "01J...",
        "type": "paragraph",
        "data": {},
        "external_id": "txt_01J...",
        "external_type": "text"
      },
      "parent_id": "root",
      "previous_id": null
    }
  ]
}
```

## Text Delta Payload

```json
{
  "text_id": "txt_01J...",
  "delta": [
    { "retain": 5 },
    { "insert": "hello", "attributes": { "bold": true } }
  ]
}
```

For the MVP, this can be stored as JSON delta. If CRDT integration is added immediately, payload can be Yrs/Yjs binary update encoded as base64 or bytes.

## Presence Payload

```json
{
  "selection": {
    "start": { "path": [0, 2], "offset": 4 },
    "end": { "path": [0, 2], "offset": 9 }
  },
  "metadata": {
    "name": "Alice",
    "cursor_color": "#2f80ed"
  }
}
```

Presence is ephemeral. Store it in memory or Redis with TTL. Do not write every cursor movement to PostgreSQL.

## Sequence Rules

- Every accepted document update gets a monotonic `server_seq` per document.
- Client must send `base_server_seq`.
- If the client is too stale, server returns `client_out_of_date`.
- Client then fetches missing updates or reboots from latest snapshot.

## Snapshot Rules

Create a snapshot when any condition is met:

- 100 accepted updates since last snapshot.
- 5 minutes since last snapshot and document changed.
- User creates a named version.
- Before restore.

Snapshot content:

```json
{
  "schema_version": 1,
  "root_block_id": "root",
  "blocks": {},
  "texts": {},
  "metadata": {}
}
```

## PostgreSQL Tables

Initial schema:

```sql
users
workspaces
workspace_members
views
documents
document_updates
document_snapshots
document_versions
files
```

Important indexes:

```text
workspace_members(workspace_id, user_id)
views(workspace_id, parent_view_id, position)
documents(workspace_id, id)
document_updates(document_id, seq)
document_snapshots(document_id, version_seq)
document_versions(document_id, created_at)
```

## Conflict Handling

MVP:

- Use server-authoritative sequencing.
- Reject stale structural operations when they cannot be safely transformed.
- Let client refetch and reapply local pending operations.

Later:

- Integrate Yrs/AppFlowy-Collab for CRDT structural updates.
- Keep server validation and permission checks around CRDT updates.

## History Semantics

History is append-only.

Restore does not delete old updates. It appends a `restore_snapshot` update and creates a fresh snapshot after restore.

This keeps auditability and makes collaboration easier.
