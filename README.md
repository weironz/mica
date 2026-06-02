# Mica

Cloud-first collaborative Markdown workspace.

## Current Scope

This repository is bootstrapping the backend-first MVP:

- Rust backend with Axum.
- PostgreSQL as the source of truth.
- REST API for auth, workspaces, page tree, and documents.
- WebSocket document rooms for real-time multi-client sync and presence.
- Append-only document history with named versions and restore.
- Markdown import, Markdown/HTML export, and S3/MinIO file uploads.
- Flutter Web client with live document sync and collaborator presence.

See:

- [Architecture](docs/architecture.md)
- [Editor Design Principles](docs/editor.md)
- [Sync and API Draft](docs/sync-and-api.md)
- [MVP Plan](docs/mvp-plan.md)

## Development

Copy environment defaults:

```sh
cp .env.example .env
```

Start PostgreSQL with Docker Compose:

```sh
docker compose up -d postgres
```

Run the API server locally:

```sh
cargo run -p mica-api-server
```

The API runs migrations automatically on startup.

The full stack can also be started through Compose when a containerized API is needed:

```sh
docker compose up --build
```

Health checks:

```sh
curl http://127.0.0.1:8080/api/health
curl http://127.0.0.1:8080/api/ready
```

Connect to a document room over WebSocket to receive and send live updates.
Authenticate with the JWT access token via the `Authorization` header or, for
browser clients, the `token` query parameter:

```text
GET /ws/workspaces/{workspace_id}/documents/{document_id}?token=<access_token>
```

On connect the server sends `document.bootstrap` (latest snapshot, current
`server_seq`, and permissions) followed by `presence.state`. Clients send
`document.update`, `presence.update`, and `ping`; the server broadcasts
`document.update.accepted`, presence changes, and `pong`. See
[Sync and API Draft](docs/sync-and-api.md) for the full message envelope.

Document history is append-only. The change log and named versions are listed
together, and a restore is recorded as a new update (broadcast to connected
clients) rather than rewriting the past:

```text
GET  /api/workspaces/{workspace_id}/documents/{document_id}/history
POST /api/workspaces/{workspace_id}/documents/{document_id}/versions
GET  /api/workspaces/{workspace_id}/documents/{document_id}/versions/{version_id}
POST /api/workspaces/{workspace_id}/documents/{document_id}/restore
```

`POST /versions` pins the current state under a name; `POST /restore` accepts
either a `version_id` or a raw `version_seq`.

Documents import from and export to Markdown, and export to HTML:

```text
POST /api/workspaces/{workspace_id}/documents/import/markdown
GET  /api/workspaces/{workspace_id}/documents/{document_id}/export/markdown
GET  /api/workspaces/{workspace_id}/documents/{document_id}/export/html
```

File uploads use a presigned, direct-to-storage flow against any S3-compatible
backend (configure the `S3_*` variables; see `.env.example`). The client
presigns, `PUT`s the bytes straight to storage, then records metadata:

```text
POST   /api/workspaces/{workspace_id}/files/presign
POST   /api/workspaces/{workspace_id}/files/complete
GET    /api/workspaces/{workspace_id}/files/{file_id}
DELETE /api/workspaces/{workspace_id}/files/{file_id}
```

When the `S3_*` variables are unset these endpoints return `503`. For local
development, run MinIO and create the bucket:

```sh
docker run -d --name mica-minio -p 9000:9000 -p 9001:9001 \
  -e MINIO_ROOT_USER=minioadmin -e MINIO_ROOT_PASSWORD=minioadmin \
  quay.io/minio/minio server /data --console-address ":9001"
docker run --rm --network host --entrypoint sh minio/mc -c \
  "mc alias set local http://127.0.0.1:9000 minioadmin minioadmin && mc mb -p local/mica"
```

Run the Flutter Web client when Flutter is installed:

```sh
cd clients/mica_flutter
flutter pub get
flutter run -d chrome --dart-define=MICA_API_BASE_URL=http://127.0.0.1:8080
```

The client opens a WebSocket document room per page: it edits over REST, and
the server's broadcast advances every other open client to the latest snapshot
and renders collaborator presence in the document header. With no
`--dart-define`, the client targets `http://<page-host>:8080`, so a release
build can be served as static files:

```sh
flutter build web
# serve build/web with any static file server, e.g.:
python3 -m http.server 8090 --directory build/web
```
