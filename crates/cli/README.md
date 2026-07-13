# mica-cli

A small, scriptable command-line client for a Mica server. It is a **thin HTTP
client over the REST API** — the symmetric counterpart to the MCP server
(`crates/mcp-server`): same capabilities, one for shells/CI, one for AI agents.
It talks to the public API only and **never touches the database**. It carries
**no backup engine** — `export` produces a portable tree and you point a
dedicated tool (restic/rustic/rclone/borg) at it (see [`docs/backup.md`](../../docs/backup.md)).

## Install

Prebuilt binaries ship with each GitHub Release, named
`mica-cli-<version>-<os>-<arch>` (Windows / Linux / macOS). Download, make
executable, put it on your `PATH`.

Or build from source (pure Rust + rustls, no OpenSSL):

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
| Server URL | `--server` | `MICA_SERVER` | flag/env override the config file |
| Access token | — | `MICA_TOKEN` | env overrides the saved token |
| Login email | `--email` | `MICA_EMAIL` | — |
| Login password | `--password` | `MICA_PASSWORD` | else read from stdin |

Stateless example (nothing saved, ideal for CI / a backup container):

```bash
MICA_SERVER=https://mica.example.com MICA_TOKEN=mica_pat_… mica-cli export --out /export
```

## Global options

Available on every command:

- `--server <URL>` — server base URL (overrides the saved config; `MICA_SERVER`).
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

## Backup

`mica-cli` deliberately has no `backup` command (that engine was retired). The
pattern is **`mica-cli export` + an external backup tool**: e.g. `rustic backup
<out>`, `rclone sync <out> remote:`, or `restic`/`borg`/`cron + tar`. The
production stack wires `export` + `rustic` into one container — see
[`docs/backup.md`](../../docs/backup.md).

## Scripting / agents

- `--json` on any command yields structured output.
- Set `MICA_SERVER` + `MICA_TOKEN` in the environment to run without any saved
  config (nothing written to disk).
- Non-zero exit on failure; error detail goes to stderr.
