# mica-cli

A small, scriptable command-line client for a Mica server. It is a **thin HTTP
client over the REST API** — the symmetric counterpart to the MCP server
(`crates/mcp-server`): same capabilities, one for shells/CI, one for AI agents.
It talks to the public API only and **never touches the database**. It carries
**no backup engine** — `export` produces a portable tree and you point a
dedicated tool (restic/rustic/rclone/borg) at it (see [`backup.md`](backup.md)).

## Install

**Already installed? Update in place:**

```bash
mica-cli update           # to the latest release
mica-cli update --check   # just say what's available, change nothing
mica-cli update --to 0.13.28
```

It talks to the GitHub release, not to a Mica server, so it works before
`auth login` and while your server is down — which is when you may most need a
newer CLI. It downloads beside the current binary, **runs `--version` on the
download before installing it** (a truncated file or the wrong architecture
fails there, while the real binary is still in place), then swaps it in.

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
setting, and `docs/mcp-connect.md` tells people to set it — so **every** command
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

### `export` — export workspaces to Markdown + images

```bash
mica-cli export --out <DIR> [--ws <WORKSPACE_ID>] [--no-prune]
```

- `--out <DIR>` — output directory. **Mirrored** by default: unchanged files are
  kept, and content removed upstream is **pruned** from the tree (so it always
  reflects current state — great for incremental backups that dedup well).
- `--ws <ID>` — export only one workspace (default: all your workspaces).
- `--no-prune` — keep files for content that no longer exists (disable mirror
  pruning).

Only GETs — a **read-scoped** token suffices.

#### Output layout

```
<out>/
  manifest.json                     # { tool, exported_at, workspaces:[{ id, name, dir, files }] }
  <workspace-slug>/                 # one dir per workspace (slug of the name;
    <page>.md                       #   id suffix only if two names collide)
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

`mica-cli` deliberately has no `backup` command (that engine was retired). The
pattern is **`mica-cli export` + an external backup tool**: e.g. `rustic backup
<out>`, `rclone sync <out> remote:`, or `restic`/`borg`/`cron + tar`. The
production stack wires `export` + `rustic` into one container — see
[`backup.md`](backup.md).

## Scripting / agents

- `--json` on any command yields structured output.
- Set `MICA_SERVER` + `MICA_PAT` in the environment to run without any saved
  config (nothing written to disk).
- Non-zero exit on failure; error detail goes to stderr.
