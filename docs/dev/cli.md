# mica-cli

A small, scriptable command-line client for a Mica server. It is a **thin HTTP
client over the REST API** — the symmetric counterpart to the MCP server
(`crates/mcp-server`): same capabilities, one for shells/CI, one for AI agents.
It talks to the public API only and **never touches the database**. It carries
**no backup engine** — `export` produces a portable tree and you point a
dedicated tool (restic/rustic/rclone/borg) at it (see [`backup.md`](../ops/backup.md)).

## Install

**Already installed? Update in place:**

```bash
mica-cli update           # to the latest release
mica-cli update --check   # just say what's available, change nothing
mica-cli update --to 0.13.28
```

It talks to the GitHub release, not to a Mica server, so it works before
`auth login` and while your server is down — which is when you may most need a
newer CLI. It downloads beside the current binary, **verifies the download
against the release's `sha256-<os>.txt` manifest**, **runs `--version` on it and
requires the exact version** (a truncated file, a tampered file, or the wrong
architecture fails there, while the real binary is still in place), then swaps
it in. It also **refuses to downgrade** — a release older than the running build
needs `--force`, so "latest" lagging behind a dev build can never quietly swap
in an older binary.

Windows note, and the reason this is not a plain overwrite: Windows refuses to
overwrite a **running** image but will happily rename it — and anyone using the
MCP server has one running. So the old binary is moved aside under a unique
name and swept on the next run. **A running MCP client keeps the old binary
until it restarts**, so restart the client to pick the new one up.

> `--to` a release older than the one that added this command leaves you with a
> binary that has no `update`. Recover with the installer line below.

**First install (or recovery).** One line, and re-running it also updates to the
latest release.

Windows (PowerShell):

```powershell
irm https://raw.githubusercontent.com/weironz/mica/main/install.ps1 | iex
```

Linux / macOS:

```bash
curl -fsSL https://raw.githubusercontent.com/weironz/mica/main/install.sh | sh
```

The script downloads the prebuilt binary from the GitHub release (nothing is
built), drops it in a per-user dir, and puts it on your `PATH`:

- Windows → `%LOCALAPPDATA%\Mica\bin\mica-cli.exe`
- Linux / macOS → `~/.local/bin/mica-cli` (override with `MICA_BIN_DIR`)

Pin a specific version instead of latest:

```powershell
$env:MICA_VERSION='0.5.4'; irm https://raw.githubusercontent.com/weironz/mica/main/install.ps1 | iex
```
```bash
MICA_VERSION=0.5.4 curl -fsSL https://raw.githubusercontent.com/weironz/mica/main/install.sh | sh
```

> Prebuilt platforms: `windows-x64`, `linux-x64`, `macos-arm64` (Apple Silicon).
> On anything else, build from source. The binary is fetched from GitHub — if
> that network is slow or blocked, prefer manual/source install below.

**Manual.** Grab `mica-cli-<version>-<platform>` from the
[GitHub Releases](https://github.com/weironz/mica/releases), make it executable,
put it on your `PATH`.

**From source** (pure Rust + rustls, no OpenSSL):

```bash
cargo build --release -p mica-cli      # → target/release/mica-cli
```

## Quick start

```bash
export MICA_SERVER=https://mica.example.com

mica-cli auth login --email you@example.com     # prompts for password, saves a token
mica-cli ws list                                 # your workspaces
mica-cli export --out ./mica-export              # every workspace → Markdown + images
```

## Configuration

State (server URL + saved token) lives in a JSON config file, written `0600` on
unix. Location — first that resolves:

1. `$MICA_CONFIG`
2. `$XDG_CONFIG_HOME/mica/config.json`
3. `$HOME/.config/mica/config.json` (unix) / `%APPDATA%\mica\config.json` (windows)

**Both fields are overridable per-invocation**, so an agent or CI run can stay
fully stateless (nothing written to disk):

| Setting | Flag | Env var | Precedence |
| ------- | ---- | ------- | ---------- |
| Server URL | `--server` | `MICA_SERVER`, or `MICA_API_BASE_URL` | legacy name first, then flag/env, then the config file |
| Access token | `--token` | `MICA_PAT` | flag > env > saved token |
| Login email | `--email` | `MICA_EMAIL` | — |
| Login password | `--password` | `MICA_PASSWORD` | else read from stdin |

`MICA_API_BASE_URL` is the standalone MCP server's historical name for the same
setting, and `docs/dev/mcp-connect.md` tells people to set it — so **every** command
accepts it, at the front of the chain. It used to be read by `mica-cli mcp`
alone: a Claude config written exactly as documented made `mica-cli ws list`
answer "no server — … set MICA_SERVER" while the URL sat right there in the
environment (reported 2026-08-28, fixed the same day). That is the token split
below, one field over; both are now one chain apiece.

`MICA_PAT` is the only token variable. There used to be two: `mcp` read
`MICA_PAT`, every other command read only `MICA_TOKEN`, so following the MCP
instructions and then running any other command reported "not logged in" without
saying which name it wanted. `MICA_TOKEN` is retired — setting it now produces an
error naming the replacement, rather than silently doing nothing.

A blank value counts as unset, not as an empty token: `MICA_PAT=` in a unit file
falls through to the next source instead of buying a 401. Surrounding whitespace
is trimmed, so `MICA_PAT=$(cat token.txt)` works.

Stateless example (nothing saved, ideal for CI / a backup container):

```bash
MICA_SERVER=https://mica.example.com MICA_PAT=mica_pat_… mica-cli export --out /export
```

## Global options

Available on every command:

- `--server <URL>` — server base URL (overrides the saved config; `MICA_SERVER`).
- `--token <TOKEN>` — API token, overriding `MICA_PAT` and the saved login. Note
  it lands in your shell history; prefer the env var for anything you type twice.
- `--json` — emit machine-readable JSON instead of human text (for scripts/agents).
- `-h, --help` / `-V, --version`.

## Commands

### `auth` — authentication

```bash
mica-cli auth login [--email <E>] [--password <P>]   # log in, save token to config
mica-cli auth whoami                                  # print the signed-in user
mica-cli auth logout                                  # forget the saved token
```

Password resolution for `login`: `--password` → `MICA_PASSWORD` → interactive
stdin prompt.

#### `auth token` — long-lived API tokens (PATs)

```bash
mica-cli auth token create --name <NAME> [--scope read|write]… [--expires-days <N>]
mica-cli auth token list                              # never shows the secret
mica-cli auth token revoke <ID>
```

- `--scope` is repeatable: `read` and/or `write` (`write` implies `read`).
  Default is `read`.
- `--expires-days` — omit for a token that never expires.
- **The token secret is printed ONCE on `create`** — capture it then (e.g. into
  a backup container's env or a password manager). A read-scoped token is all a
  backup/export needs.

### `ws` — workspaces

```bash
mica-cli ws list        # list your workspaces (add --json for scripts)
```

### `page` — pages and folders

```bash
mica-cli page list --ws <WS> [--parent <VIEW>] [--depth <N>] [--limit <N>] [--offset <N>] [--with-stats]
mica-cli page read --ws <WS> <DOC_ID>...
mica-cli page move --ws <WS> [--to <FOLDER_VIEW>] <VIEW_ID>...
mica-cli page trash --ws <WS> --confirm <VIEW_ID>...
```

**Two ids, and they are not interchangeable.** `page list` prints both: `view=`
addresses a node in the tree (move, trash, restore), `object=` addresses the
document it points at (read). Reaching for the wrong one is the usual mistake.

`move` and `trash` take **many ids and send one request** — that is the point of
them. A folder carries its whole subtree, so pass the folder rather than listing
its children. They report what actually happened:

```
320 trashed (subtrees included).
2 requested id(s) were skipped — already in that state, or gone:
```

The count includes descendants, so it is normally larger than the list you
passed; never read your own request size back as the result.

`page read` with several ids uses the batch endpoint (one round trip). A page
that cannot be read is named on stderr and the rest still print. With exactly
one id the output is the bare Markdown, so `page read … > out.md` works.

`--with-stats` adds each page's stored size. **Small means nearly empty** — that
is how you find stub pages without reading every one of them. Large does NOT
mean long: deleted text leaves weight behind, so it is not a word count.

### `trash` — the recycle bin

```bash
mica-cli trash list --ws <WS>
mica-cli trash restore --ws <WS> <VIEW_ID>...      # bulk undo, subtrees included
mica-cli trash empty --ws <WS> --confirm           # PERMANENT
```

`page trash` is reversible with `trash restore`; `trash empty` is not reversible
at all. Both ask for `--confirm`, and only one of them can be taken back.

### `import` — import archives

```bash
mica-cli import <ARCHIVE>... [--ws <WS>] [--parent <VIEW>] [--name <NAME>] [--notion] [--no-rehost]
```

Each archive is watched to completion and a bad archive does not stop the rest.
The summary reports the server's **true totals** (`skipped_total`,
`image_failures_total`), not the length of the capped sample lists; when the
sample is shorter it says how many were listed. External images that could not
be stored are named individually (page / document / block / url / reason) —
the page still links to the origin, so treat that import as incomplete.
`--json` keeps the same facts as structured fields: `skipped` (listed count),
`skipped_total`, `image_failures[]`, `image_failures_total`.

### `rehost-images` — pull external image links into Mica

```bash
mica-cli rehost-images [--ws <WS>] [--remap <FILE>] [--drop <FILE>]
```

Every item is attempted even after a failure: partial success is kept and
summarized on stdout (`rehosted` / `dropped` / `failed`). **If any request
failed the process exits non-zero**, and per-item failure detail goes to stderr
(also under `--json`, where the stdout summary stays JSON).

### `export` — export workspaces to Markdown + images

```bash
mica-cli export --out <DIR> [--ws <WORKSPACE_ID>] [--no-prune] [--full]
```

- `--out <DIR>`：保存一份完整的 Markdown、结构 sidecar 与图片镜像。默认清理源端已删除的内容。
- `--ws <ID>`：只更新这个工作区，保留输出目录中其他工作区的文件和顶层清单记录。
- `--no-prune`：保留源端已删除的文件；若旧文件与新目录发生路径冲突，会明确失败，避免偷偷删除。
- `--full`：忽略本地版本记录，重新获取所有页面；仍保持相同内容文件的修改时间。

**默认执行源端增量导出**：首次获取全部页面；后续先读取版本清单，只请求新增、正文变化、
引用附件元数据变化，或本地文件缺失/修改的页面。目录改名、移动、页面增删等会使该工作区
重新导出，以重建文件路径与跨页相对链接。新增无关附件不会让其他页面失效。
正文变化时，已经验证本地内容和服务端版本的附件会复用，不必再次从对象存储下载；
同一轮其他页面已下载的共享附件也可复用。每次请求最多声明 64 个可复用附件，优先当前页
上次引用的附件；超出的附件正常下载，结果仍完整。

版本记录放在 `<out>/.mica-export/`，按服务器 URL、工作区 ID 和导出协议版本隔离，不保存凭据。
缓存缺失、损坏、上轮中断时自动重建；操作系统文件锁防止两个导出进程写同一个目录，
**运行中不要删除 `.mica-export/lock`**。缺少新接口的旧服务端会明确失败，需要同时升级服务端与 CLI。

每次只下载并解开一个页面的 ZIP。变化内容先暂存到磁盘，完整下载且目录版本校验成功后，
逐文件原子替换，再更新成功版本记录和清理旧文件。失败不创建成功版本；安装期间中断时，
下次重建该工作区。未变化文件不改写，适合继续交给 rustic 去重快照。

资源边界：清单由 PostgreSQL 扫描权威 CRDT state 的摘要，不解码正文、不下载附件，
也不把全工作区 state 传给 API 进程；它仍有数据库扫描成本。内存以单页及该页附件总量为界，
一个页面包含巨大附件时仍可能占用较多内存。暂存空间只保留当前工作区的变化文件。

`--json` 结果额外包含 `pages_exported`、`pages_skipped`、`bytes_downloaded`、`assets_reused`，可直接验证
是否跳过源端导出，而不必从本地文件修改时间猜测。

Only GETs — a **read-scoped** token suffices.

#### Output layout

```
<out>/
  manifest.json                     # { tool, exported_at, workspaces:[{ id, name, dir, files }] }
  .mica-export/                     # local version cache + OS lock; safe to rebuild when idle
  <workspace-slug>/                 # one dir per workspace (slug of the name;
    <page>.md                       #   id suffix only if two names collide)
    <page>.mica.json                 # block structure + original links + asset mapping
    <folder>/<page>.md              # nested folders preserved
    assets/…                        # referenced images, original filenames
```

> The workspace dir is a **slug of the name** (ASCII only; a non-ASCII name like
> a Chinese one slugs to empty → falls back to `workspace-<id8>`). `manifest.json`
> is the authoritative `id → name → dir` map — key automation off the **id**, not
> the dir name, since names can be renamed or non-ASCII.

### `mcp` — Model Context Protocol server (for AI clients)

`mica-cli mcp` serves the Mica tools to any MCP client (Claude Code / Desktop,
Cursor, Codex, Gemini, Windsurf …) over stdio. See [`mcp-connect.md`](mcp-connect.md)
for the full tool list.

```bash
mica-cli mcp [--read-only]        # serve over stdio (what the client launches)
```

#### Registering it with a client

Use the client's own command — there is deliberately no `mica-cli mcp install`
(see [`mcp-connect.md`](mcp-connect.md) for why it was removed). For Claude Code:

```bash
mica-cli auth login --server https://your-server.example.com --email you@example.com
claude mcp add --scope user mica -- /path/to/mica-cli mcp
```

Passing no `-e` is the cleanest form: the server walks the same credential chain
as every other subcommand, so **no token lands in the client config or in your
shell history**. The trade is that it rides your saved login, which can expire —
re-run `auth login` if the MCP server starts reporting `not signed in`.

To pin a token instead (copying the config to another machine, or not wanting to
depend on a saved login):

```bash
claude mcp add --scope user mica \
  -e MICA_API_BASE_URL=https://your-server.example.com \
  -e MICA_PAT=mica_pat_… \
  -- /path/to/mica-cli mcp
```

> `--scope user` matters on Windows: the default `local` scope keys the entry by
> the current directory, and the CLI writes that key with forward slashes while
> the app uses backslashes — so the two never see each other's config. Symptom:
> `claude mcp list` says `✓ Connected` while the app's session has no `mica_*`
> tools at all. See [`mcp-connect.md`](mcp-connect.md).

## Backup

Two different things share the word, and the doc used to claim the second did
not exist ("`mica-cli` deliberately has no `backup` command") while the
production stack's container entrypoint *was* that command:

- **`mica-cli export`** — every workspace as Markdown + images, for you to point
  an external tool at: `rustic backup <out>`, `rclone sync <out> remote:`,
  `restic`/`borg`/`cron + tar`.
- **`mica-cli backup run` / `backup daemon`** — the scheduled off-site backup
  that carries content *plus* the database *plus* object bytes. `run` does one
  pass and exits non-zero if it covered less than it should; `daemon` runs one
  on start and then daily at `BACKUP_HOUR`. See [`backup.md`](../ops/backup.md).

The reason the second is code and not the `mica-backup.sh` pair it replaced: a
shell run is just an exit code, so "backed up less than you think" and "backed
up everything" looked identical — and did, in production, for months.

### The `mica-cli` Docker image

`ENTRYPOINT` is `mica-cli backup daemon`, because the image exists to be the
production stack's backup sidecar. **To use it as an ordinary CLI, override the
entrypoint** — otherwise every argument is parsed as a flag of `backup daemon`
and you get `error: unexpected argument 'ws' found`:

```bash
docker run --rm --entrypoint mica-cli \
  -e MICA_SERVER=https://your-server.example.com -e MICA_PAT=mica_pat_… \
  <registry>/mica-cli:v0.13.37 ws list
```

## Scripting / agents

- `--json` on any command yields structured output.
- Set `MICA_SERVER` + `MICA_PAT` in the environment to run without any saved
  config (nothing written to disk).
- Non-zero exit on failure; error detail goes to stderr.
