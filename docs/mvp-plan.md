# MVP Development Plan

## MVP Definition

The MVP is a cloud-first collaborative document editor with:

- User login.
- Workspace creation.
- Page tree.
- Block document editor.
- Real-time multi-client sync.
- History list and restore.
- Markdown import/export.
- File/image upload.

Explicitly out of scope:

- Local-first mode.
- Offline conflict resolution.
- Infinite canvas.
- Full Notion-style database.
- AI.
- Public publishing.
- Mobile polish.

## Phase 0: Repo Bootstrap

Duration: 2-4 days.

Deliverables:

- Rust workspace.
- Axum server.
- Config and tracing.
- SQLx PostgreSQL connection.
- Migration runner.
- Docker Compose for PostgreSQL.
- Flutter app skeleton.

Done when:

- Server boots.
- Health endpoint works.
- Database migration runs.
- Flutter app can call `/api/health`.

## Phase 1: Auth and Workspace

Duration: 1 week.

Deliverables:

- Register/login.
- Current user endpoint.
- Workspace CRUD.
- Workspace membership.
- Basic role checks.

Done when:

- User can create a workspace.
- User can invite or add another user in dev mode.
- API rejects unauthorized workspace access.

## Phase 2: Page Tree

Duration: 1 week.

Deliverables:

- View/page model.
- Create document view.
- Rename view.
- Move/reorder view.
- Soft delete view.

Done when:

- Flutter sidebar can render page tree.
- Creating a page creates both `view` and `document`.

## Phase 3: Document Model

Duration: 1-2 weeks.

Deliverables:

- Document snapshot schema.
- Block model.
- Operation validation.
- Apply insert/update/delete/move block.
- Text delta storage.
- Markdown export for basic blocks.

Done when:

- API can create a document.
- API can apply block operations.
- API can return current document snapshot.
- Markdown export returns useful text.

## Phase 4: Real-time Sync

Duration: 2 weeks.

Deliverables:

- WebSocket document room.
- Authentication on WebSocket.
- Per-document server sequence.
- Broadcast accepted updates.
- Presence messages.
- Reconnect bootstrap.

Done when:

- Two browser/client windows edit the same document.
- Updates appear in both clients without refresh.
- Stale client can reconnect and catch up.

## Phase 5: History

Duration: 1 week.

Deliverables:

- Append-only `document_updates`.
- Periodic `document_snapshots`.
- History listing.
- Named version creation.
- Restore snapshot.

Done when:

- User can see previous versions.
- User can restore a previous version.
- Restore appears as a new update for other connected clients.

## Phase 6: Flutter Editor PoC

Duration: 2-3 weeks.

Deliverables:

- Flutter document page.
- Editor state model.
- Transaction-to-operation adapter.
- WebSocket sync client.
- Remote update application.
- Presence rendering if feasible.

Initial block support:

- paragraph
- heading
- todo
- bulleted list
- numbered list
- quote
- code block

Done when:

- Editing in Flutter produces backend operations.
- Remote backend operations update Flutter editor state.
- Basic Markdown shortcuts work.

## Phase 7: Import, Export, Files

Duration: 1-2 weeks.

Deliverables:

- Markdown import.
- Markdown export.
- HTML export.
- S3/MinIO upload flow.
- Image/file block support.

Done when:

- User can import a Markdown file.
- User can export the document back to Markdown.
- User can insert an image.

## Suggested First Code Milestone

Start with backend first:

```text
GET  /api/health
POST /api/auth/register
POST /api/auth/login
POST /api/workspaces
POST /api/workspaces/{workspace_id}/documents
GET  /api/workspaces/{workspace_id}/documents/{document_id}/bootstrap
GET  /ws/workspaces/{workspace_id}/documents/{document_id}
```

This proves the cloud-first foundation before committing heavily to Flutter editor internals.

## Engineering Rules

- All writes go through service methods, not directly from handlers to SQL.
- All WebSocket writes run the same permission checks as REST writes.
- Document history is append-only.
- Snapshot schema has an explicit `schema_version`.
- Frontend editor is behind an adapter so editor implementation can change.
- Markdown import/export must have tests.
- Operation application must have tests.

## Test Plan

Backend unit tests:

- operation validation
- operation application
- snapshot restore
- permission checks
- Markdown conversion

Backend integration tests:

- register/login/workspace flow
- create document
- WebSocket connect
- two clients update same document
- restore broadcasts update

Frontend tests:

- transaction adapter
- document state reducer
- WebSocket reconnect behavior

## Open Technical Questions

1. Should MVP structural operations be custom JSON ops or Yrs/AppFlowy-Collab updates from day one?
2. Should Flutter be the Web MVP, or should Web use a mature DOM editor first?
3. Should document snapshots store only full JSON state, or both JSON state and encoded CRDT state?
4. Should comments be in MVP history or a separate post-MVP module?

Recommended answers:

1. Start with custom JSON ops, keep a CRDT-compatible envelope.
2. Flutter is acceptable if AppFlowy alignment matters more than fastest Web-editor maturity.
3. Store JSON state first; add encoded CRDT when the collab layer is introduced.
4. Keep comments post-MVP.
