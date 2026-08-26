# mica-mcp-server — design notes

Why the MCP server is shaped the way it is. **To set it up, read
[mcp-connect.md](mcp-connect.md)** — that is the authoritative guide and the one
kept current; this file is the rationale behind it.

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

`mica-cli mcp` — the MCP server is a library folded into the CLI, so CI ships one
binary per platform rather than two. There is no separate `mica-mcp` executable
(there was, briefly; it was never published, which is exactly why it merged).

It resolves the server URL and PAT through the CLI's usual chain — `--server` /
`MICA_SERVER`, `MICA_PAT`, or a saved `mica-cli auth login` — and still honours
the historical `MICA_API_BASE_URL` / `MICA_PAT` so existing MCP configs keep
working. `MICA_MCP_READ_ONLY=1` registers the write tools but refuses them at
call time.

Full setup, client config, tool list and troubleshooting: **[mcp-connect.md](mcp-connect.md)**.

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
  requires `confirm: true`. `destructiveHint`.

### 删除这条链(每一档都要 `confirm: true`)

~~「Permanent purge is **not** exposed」~~ —— 这句在这里挂了很久,而 `mica_purge_view`
和 `mica_empty_trash` 早就存在了。**否定式声明只会静默变假**:代码删个函数会编译报错,
文档说「没有」而实际有了,什么都不会响。

| 想做什么 | 工具 | 可恢复? |
| --- | --- | --- |
| 删一个 / 一批页面 | `mica_trash_view` / `mica_trash_views` | ✅ 进回收站 |
| 撤销 | `mica_restore_view` / `mica_restore_views` | — |
| 永久删一个 | `mica_purge_view` | ❌ |
| **永久删一批** | **`mica_purge_views`** | ❌ |
| 清空回收站 | `mica_empty_trash` | ❌ |
| **删掉整个工作区** | **`mica_delete_workspace`** | ❌ **没有工作区回收站** |

两条值得单独记住的边界:

- **`mica_purge_views` 只作用于已经在回收站里的条目**,没进回收站的原样返回在 `skipped`
  里。这是和单个 `mica_purge_view` **有意的不同** —— 后者不做这道过滤,能把一个活着的
  页面直接永久删掉。把那个行为批量化,等于让一次调用能抹掉几百个从没被删过的页面,
  中间没有任何可恢复的一步:那是**新增了一种能力**,不是把旧能力做快。
- **`mica_delete_workspace` 要 `confirm: true` 外加 `expect_name`** 与工作区当前名字
  完全一致,对不上就拒绝且不动任何东西。这不是仪式:它是这里唯一会摧毁「从未被删过的
  内容」的工具,而一个手里攥着三十个工作区 UUID 的 agent,写错一位就没有回收站可捞。
  id 用眼睛核不了,名字才是用户真正说出口的那个东西。

Note the two id kinds: reads/writes use a page's **`document_id`** (its
`object_id`, from `mica_list_pages`); move/trash use its **`view_id`**.

## External backup

Backups are deliberately **not** a Mica feature, and the MCP server does not do
them either — it is a thin proxy with no storage of its own. Export and point a
real backup tool at the result: see **[backup.md](backup.md)**.
