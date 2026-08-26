# HTTP API

Every endpoint the server serves, taken from the router in
[`crates/api-server/src/routes/mod.rs`](../crates/api-server/src/routes/mod.rs).

There is no generated spec yet, so this file is hand-maintained — **add a route,
add a line here**. It exists because the alternative turned out to be worse than
nothing: the endpoint lists in [`sync-and-api.md`](sync-and-api.md) are a design
draft naming routes that were never built, so following them produced 404s and
sent one person to `strings` on the binary to discover that reading a page is
spelled `export`. A wrong map costs more than no map.

## Basics

- Base path `/api` for everything below except the WebSocket, share, health and
  mail-link routes, which are marked.
- Auth is `Authorization: Bearer <token>` — a session token from `auth/login` or
  a PAT from `auth/tokens`. Public routes are marked **public**.
- Errors are JSON: `{"code": "not_found", "message": "..."}`. **A path that does
  not exist answers the same way**, so a 404 with a body means "no such route" —
  probing is safe and tells you something.
- Ids: a **view id** addresses a node in the page tree (move / trash / restore);
  an **object id** addresses the document it points at (read / write). They are
  different uuids for the same page. Listings return both.

## Reading and writing a page

The two you want first:

```
GET   /api/workspaces/{workspace_id}/documents/{document_id}/markdown
PATCH /api/workspaces/{workspace_id}/documents/{document_id}/markdown
```

`GET .../export/markdown` is the same handler under the older name, still valid.

## Bulk operations

The per-item routes are fine for one thing at a time. Reorganising a workspace
is not that: these take a list and send one request.

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/workspaces/{ws}/views/batch-trash` | Soft-delete many views, each with its subtree |
| POST | `/workspaces/{ws}/views/batch-restore` | Bring many back out of the bin |
| POST | `/workspaces/{ws}/views/batch-move` | Re-parent many under one folder, in order |
| POST | `/workspaces/{ws}/views/batch-purge` | **Permanently** delete many views **already in the bin** |
| POST | `/workspaces/{ws}/documents/batch-read` | Read many pages' Markdown in one round trip |
| POST | `/workspaces/{ws}/views/reorder` | Set the full child order under one parent |

Body is `{"view_ids": [...]}` (`document_ids` for the read, plus
`parent_view_id` for the move). Trash / restore / move / purge answer:

```json
{ "affected": 320, "skipped": ["<uuid>"] }
```

**`affected` is what happened, not what you asked for** — it counts descendants
pulled along with a folder, so it is normally larger than the list you sent.
`skipped` names requested ids that were already in that state, or gone. Never
read your own request size back as the result.

`batch-read` answers `{"documents": [{document_id, markdown | error}]}` — a page
that cannot be read reports inline instead of failing the whole survey.

⚠️ **`batch-purge` acts only on views that are ALREADY trashed**, and that is a
deliberate difference from the single `DELETE /trash/{view_id}`, which has no
such filter and will permanently erase a live page. Batching *that* would let
one request destroy hundreds of pages nobody had deleted, with no recoverable
step anywhere. An id that is not in the bin comes back in `skipped` and is left
untouched — so a real cleanup is always trash first, then purge.

## Page tree

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/workspaces/{ws}/views` | The page tree. Filters below |
| PATCH | `/workspaces/{ws}/views/{view_id}` | Rename / set icon |
| DELETE | `/workspaces/{ws}/views/{view_id}` | Trash one view + subtree. Returns the remaining tree |
| POST | `/workspaces/{ws}/views/{view_id}/move` | Re-parent one view |
| POST | `/workspaces/{ws}/views/{view_id}/clone` | Duplicate |
| POST | `/workspaces/{ws}/views/{view_id}/transfer` | Move to another workspace |
| POST | `/workspaces/{ws}/views/{view_id}/restore` | Undo a trash, with its subtree |
| GET | `/workspaces/{ws}/views/{view_id}/backlinks` | Pages linking here |
| GET | `/workspaces/{ws}/graph` | Link graph |
| POST | `/workspaces/{ws}/documents` | Create a page |
| POST | `/workspaces/{ws}/folders` | Create a folder |

`GET /views` takes optional query parameters; with none it returns the whole
tree exactly as it always did.

- `parent_view_id` — list only what is under this view
- `depth` — levels to descend, `1` = direct children only
- `limit` / `offset` — paging. **No default limit**: omitting it returns
  everything rather than a silently truncated page
- `with_stats=true` — adds `state_bytes` per page. **Small means nearly empty**,
  which is how you find stub pages without reading every one of them. Large does
  NOT mean long: a CRDT keeps deleted text as tombstones, so this is not a word
  count

**Only a folder may hold children.** Any route that sets `parent_view_id`
rejects a page as parent with a 400 and a readable reason.

## Recycle bin

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/workspaces/{ws}/trash` | What is in the bin |
| DELETE | `/workspaces/{ws}/trash` | **Empty it. Permanent** |
| DELETE | `/workspaces/{ws}/trash/{view_id}` | **Purge one subtree. Permanent** |

Trashing is a soft delete and reversible; purging is neither.

## Documents, history, comments

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/workspaces/{ws}/documents/{doc}/bootstrap` | CRDT snapshot the editor opens with |
| POST | `/workspaces/{ws}/documents/{doc}/updates` | Apply CRDT updates |
| GET | `/workspaces/{ws}/documents/{doc}/outline` | Headings + block ids + `seq`, cheap |
| POST | `/workspaces/{ws}/documents/import/markdown` | Create a page from Markdown |
| POST | `/workspaces/{ws}/documents/{doc}/rehost-image` | Store an image's bytes and repoint a block |
| GET | `/workspaces/{ws}/documents/{doc}/history` | Edit history |
| POST | `/workspaces/{ws}/documents/{doc}/versions` | Name a restore point |
| GET | `/workspaces/{ws}/documents/{doc}/versions/{version_id}` | One version |
| POST | `/workspaces/{ws}/documents/{doc}/restore` | Roll back to a version |
| GET/POST | `/workspaces/{ws}/documents/{doc}/comments` | List / create a thread |
| DELETE | `/workspaces/{ws}/documents/{doc}/comments/{thread_id}` | Delete a thread |
| POST | `/workspaces/{ws}/documents/{doc}/comments/{thread_id}/reply` | Reply |
| POST | `/workspaces/{ws}/documents/{doc}/comments/{thread_id}/resolve` | Resolve |

## Search

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/workspaces/{ws}/search` | Search one workspace (names and body text) |
| GET | `/search` | Search every workspace you belong to |

## Export

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/workspaces/{ws}/documents/{doc}/export.zip` | One page + assets |
| GET | `/workspaces/{ws}/documents/{doc}/export/html` | One page as HTML |
| GET | `/workspaces/{ws}/views/{view_id}/export.zip` | One folder |
| GET | `/workspaces/{ws}/export.zip` | One workspace |
| GET | `/workspaces/{ws}/export/markdown` | One workspace as Markdown |
| GET | `/workspaces/export.zip` | Every workspace |
| GET | `/workspaces/export/stats` | Sizes, for a progress bar |
| GET | `/export/markdown` | Everything as Markdown |

## Workspaces and members

| Method | Path | Purpose |
| --- | --- | --- |
| GET/POST | `/workspaces` | List / create |
| GET/PATCH/DELETE | `/workspaces/{ws}` | Read / rename / delete |
| POST | `/workspaces/reorder` | Sidebar order |
| GET | `/workspaces/{ws}/usage` | Bytes used |
| GET/POST | `/workspaces/{ws}/members` | List / invite |
| PATCH/DELETE | `/workspaces/{ws}/members/{user_id}` | Change role / remove |

## Import

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/workspaces/import` | Start an archive import (body is the archive) |
| GET | `/import/jobs/{job_id}` | Poll progress |
| POST | `/import/jobs/{job_id}/cancel` | Cancel |

The import runs as a background task on the server: the POST returns a job id
as soon as the archive is uploaded, and **the client can leave**. Cancelling
stops it between pages and does NOT roll back — the job reports how many pages
had already landed.

> ### ⚠️ Do not deploy while an import is running
>
> Job state lives in memory (`state.import_jobs`) and the work is a spawned
> task, so restarting the api **kills a running import** and loses its record.
> The pages already written stay, which is the bad part: the workspace is left
> holding part of an archive with nothing saying so, and the job id you were
> polling returns nothing. Wait for it to finish, or cancel deliberately (which
> at least reports what landed). This is also why "reopen the app and pick the
> progress back up" is not offered — it would work only until the next deploy.

#### Why an import can be slow, and what the numbers are

Import time is usually **not** about the page count. It is about images the
archive links to but does not contain: those are fetched from the open internet,
and a server that cannot reach them pays a timeout for each one. A CN-hosted
server routinely cannot reach the CDNs a wiki links to.

These are constants, not settings. They are written down because knowing what
the import is waiting on is the part you actually need — and because every knob
here would also have to be threaded through the compose allowlist to reach the
process at all (see [`deploy.md`](deploy.md)). If a default turns out to be
wrong for real deployments, the fix is to change the default.

| Behaviour | Value | Where |
| --- | --- | --- |
| External image fetch timeout | 8s | `routes/files.rs` |
| Images fetched at once | 8 | `REHOST_CONCURRENCY`, `routes/import.rs` |
| Timeouts before a host is written off | 2 | `HostBreaker::LIMIT`, `routes/import.rs` |
| Ids per batch endpoint | 1000 | `MAX_BATCH_VIEWS`, `routes/documents.rs` |

After two timeouts against one host, the rest of that host's images are skipped
**unattempted** and keep their original links; the count is logged. Nothing is
lost — re-run `mica-cli rehost-images` from a network that can reach them, which
is what that command is for. Or set `rehost_external=false` on the import to
skip the step entirely and do it later.

Only timeouts trip the breaker. A 404 is about one image and says nothing about
the next, so it does not count — and in real archives the two arrive mixed
together.

#### Export

Export has no equivalent tuning because it has no external network: it reads
this instance's own database and object storage. It does assemble the whole
archive in memory before responding, so a very large workspace is a memory
spike rather than a slow trickle — a different failure (OOM), with different
symptoms, and not one that has been observed in practice.

## Files and images

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/workspaces/{ws}/files/presign` | Get a presigned upload URL |
| POST | `/workspaces/{ws}/files/complete` | Record an upload |
| POST | `/workspaces/{ws}/files/resolve` | file ids → download URLs |
| POST | `/workspaces/{ws}/files/import-url` | Server-side fetch of a remote image |
| GET | `/workspaces/{ws}/files/{file_id}` | File metadata |
| DELETE | `/workspaces/{ws}/files/{file_id}` | Delete a file record |
| GET | `/workspaces/{ws}/files/{file_id}/blob` | **public** — the bytes; the link never expires |
| GET | `/workspaces/{ws}/files/{file_id}/blob/{filename}` | Same, with a cosmetic filename |

Object keys are content-addressed, so storing identical bytes twice returns the
same file — and costs quota once.

## Auth

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/auth/register` | **public** — registration, off unless enabled |
| POST | `/auth/login` | **public** |
| POST | `/auth/refresh` | **public** — rotate the session token |
| POST | `/auth/logout` | End the session |
| GET/PATCH/DELETE | `/auth/me` | Profile / update / delete the account |
| PUT/DELETE | `/auth/me/avatar` | Set / clear the avatar |
| GET | `/users/{user_id}/avatar` | **public** — an avatar's bytes |
| POST | `/auth/password` | Change password |
| POST | `/auth/password/forgot` | **public** — request a reset mail |
| POST | `/auth/resend-verification` | **public** |
| GET/POST | `/auth/tokens` | List / create PATs. **The secret prints once** |
| DELETE | `/auth/tokens/{id}` | Revoke a PAT |

A read-scoped PAT is enough for every GET here, including a full export.

## AI

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/ai/complete` | One-shot completion |
| GET/PATCH | `/ai/settings` | Read / change AI settings |

## Not under `/api`

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/ws/workspaces/{ws}/documents/{doc}` | **WebSocket** — the document room; auth by query token |
| GET | `/ws/ai` | **WebSocket** — streaming AI |
| GET | `/s/{token}` | **public** — a shared page, rendered server-side |
| GET | `/health` | **public** — reports the running version |
| GET | `/ready` | **public** — readiness |
| GET | `/metrics` | Prometheus. Not proxied publicly — reachable only inside the compose network |

## See also

- [`cli.md`](cli.md) — `mica-cli`, a thin client over these endpoints
- [`mcp-connect.md`](mcp-connect.md) — the same capabilities as MCP tools
- [`sync-and-api.md`](sync-and-api.md) — the sync **design**. Its endpoint lists
  are historical; this file is the current one
