# mica-mcp-server

An [MCP](https://modelcontextprotocol.io) server that exposes Mica as tools an AI
(Claude Desktop, Claude Code, or any MCP client) can call — list, read, create,
and write documents over the Mica REST API.

It is a **thin proxy**: it holds a Mica personal access token (PAT) and talks to
the Mica API over HTTPS. It has no database or storage access of its own, and it
does **not** do backups — see [External backup](#backup) below.

## Design

- **One capability surface.** Everything goes through the Mica REST API. The MCP
  server adds nothing but tool schemas + safety gates.
- **Markdown in, markdown out.** Writes take Markdown; the *server* derives the
  block/CRDT ops (`import_markdown` → `apply_document_operations`). The AI never
  constructs raw block ops — this is far more token-efficient and less error-prone
  (the same choice Notion's and Obsidian's MCP servers make).
- **Get outline, then patch.** `mica_get_outline` returns headings + block ids so
  a write can target a spot (`insert_at`, `find_replace`) instead of rewriting the
  whole page.

## Configure

Environment:

| Var | Required | Meaning |
| --- | --- | --- |
| `MICA_API_BASE_URL` | yes | e.g. `https://mica.cloudcele.com` |
| `MICA_PAT` | yes | a Mica personal access token (Settings → API Tokens) |
| `MICA_MCP_READ_ONLY` | no | `1`/`true` registers writes but refuses them |

Build: `cargo build --release -p mica-mcp-server` → `target/release/mica-mcp`.

### Claude Code

```bash
claude mcp add mica -- \
  env MICA_API_BASE_URL=https://mica.cloudcele.com MICA_PAT=<token> \
  /path/to/mica-mcp
```

### Claude Desktop (`claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "mica": {
      "command": "/path/to/mica-mcp",
      "env": {
        "MICA_API_BASE_URL": "https://mica.cloudcele.com",
        "MICA_PAT": "<token>"
      }
    }
  }
}
```

## Tools

Read (safe, `readOnlyHint`):

- `mica_list_workspaces` — workspaces (id, name, role).
- `mica_list_pages(workspace_id)` — the page tree (documents + folders).
- `mica_search(workspace_id, query)` — pages by title.
- `mica_read_document(workspace_id, document_id)` — content as Markdown.
- `mica_get_outline(workspace_id, document_id)` — headings + block ids.
- `mica_export_workspace(workspace_id)` — the whole workspace as Markdown.

Write:

- `mica_create_document(workspace_id, name, markdown?, parent_view_id?)`.
- `mica_update_document(workspace_id, document_id, mode, …)` — `mode` is `append`
  (safe default), `replace_all`, `insert_at` (needs `anchor`), or `find_replace`
  (needs `find`/`replace`). `destructiveHint`.
- `mica_move_document(workspace_id, view_id, parent_view_id?)`.
- `mica_trash_view(workspace_id, view_id, confirm)` — soft delete (recycle bin);
  requires `confirm: true`. `destructiveHint`. Permanent purge is **not** exposed.

Note the two id kinds: reads/writes use a page's **`document_id`** (its
`object_id`, from `mica_list_pages`); move/trash use its **`view_id`**.

## <a name="backup"></a>External backup

Backups are deliberately **not** a Mica feature. Export your data and point a
real backup tool at it — Markdown is text, so it dedups and diffs beautifully:

```bash
# dump every workspace to Markdown, then let restic/rclone/borg do the backup
mica-cli export --out ./mica-export
restic -r s3:… backup ./mica-export      # or: rclone sync ./mica-export remote:mica
```

Run it on a cron/systemd timer. Restore = `mica-cli import ./mica-export/<ws>.zip`
(the AI-facing equivalent is per-page `mica_create_document` / `mica_update_document`).
