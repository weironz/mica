# 0.14.0 deployment and acceptance

Date: 2026-10-30. Release tag `v0.14.0` points to `590df2c8`. This is the
first release shipping the single-source Markdown engine (S0–S5): the Dart
mirror is gone; parsing, serialization and HTML exist only in Rust
`crates/md-core` (crate `mica-md`), reached by desktop FFI and web wasm
(`crates/md-wasm`).

## Gates and artifacts

- `just ship 0.14.0` passed end to end on Windows (`SHIP_EXIT=0`): version
  bump, full release gate, `chore(release): 0.14.0`, tag, push of `main` and
  the tag, Windows installer (with install + launch smoke), Windows CLI,
  static Linux CLI cross-built in `rust:1.98.0-slim-bookworm`, and all three
  images pushed to ACR and Docker Hub.
- Gate receipts: rust suites all green (`receipt: 1154 passed`, floor 500);
  mica-core gated store `receipt: 213 passed`; wire gold `receipt: 9 passed`;
  wasm `mica_md` 203,116 B gzipped against the 262,144 B budget; Flutter
  analyze 0 error / 0 warning; Flutter unit tests `receipt: 2054 passed`
  (floor 1000); web bundle built (dart2js).
- Published assets: `Mica-Setup-0.14.0.exe`, `mica-cli-0.14.0-windows-x64.exe`,
  `mica-cli-0.14.0-linux-x64`, `sha256-windows.txt`, `sha256-linux.txt`.
  The release is public.
- No SQL migrations between v0.13.69 and v0.14.0 — the deploy preflight
  reported "no migrations between v0.13.69 and v0.14.0 — no restore point
  needed".

## Deployment

- `just deploy-prod 0.14.0 --check --diff` rehearsed against the real node:
  35 ok, 0 failed; changes limited to version-dependent pieces (`.env`
  render, image pull, profile-gated backup sidecar). Compose file unchanged;
  shared Traefik was not touched.
- `just deploy-prod 0.14.0` deployed: `ok=35 changed=4 failed=0`. The play's
  own verification reported `{"status":"ready","service":"mica-api-server","version":"0.14.0"}`,
  `index: 200`, bundle digest `538ed18c2bd4…`.

## Post-deployment verification

- `GET /api/health` → `{"status":"ok","service":"mica-api-server","version":"0.14.0"}`.
- `GET /mica_md/mica_md_bg.wasm` → 200, `Content-Type: application/wasm`,
  526,906 B.
- Production login page renders the new bundle (footer shows v0.14.0) and the
  page loads with no console errors.

## Web smoke

Production registration is closed and there is no shared production account,
so the interaction smoke ran against the local `just dev` stack — same 0.14.0
code and the same wasm bundle — with production covered by the checks above.

- Pasting multi-line Markdown (heading, bold, inline code, link, bullet list,
  GFM table) produced structured blocks; the table renders with header
  styling.
- Copying a page with bold + link yields rich HTML
  (`<h1>/<strong>/<code>/<a href>/<ul>/<table>`) and clean plain text (tabs
  between table cells), verified by pasting into an external contenteditable
  target.
- Table cell marks survive a full app reload (server round-trip): a cell
  containing `**葡萄**` still renders bold after reload, and a bare
  `https://example.com/x` cell still renders as a link. This exercises the new
  cell storage model (source string vs `{text, marks}`) end to end.
- `e2e/clipboard_e2e.mjs` against the harness bundle: all 24 assertions pass,
  including the cell editing sequence in both clipboard modes (open cell,
  Ctrl+A, Ctrl+B, Esc, copy → real `<strong>` in the HTML flavor; undo
  restores, redo reapplies).
- `e2e/web_e2e.mjs`: all 14 assertions pass, including two web sessions
  converging through the real server and the edit from session A arriving in
  session B.
- AI "current page" context: `ai_current_page_context_test.dart` (7 tests)
  proves the request carries the live page body and title; no live model
  round-trip ran (no provider key configured on the dev stack).

## Fix found during the smoke

`yjs_probe_web.dart` (the browser sync self-test hook) built its WebSocket URL
by hand without the `v` protocol parameter; since op-model retirement the
server refuses versionless clients with `client_too_old`, which surfaced as a
bogus convergence failure in `web_e2e`. The probe now uses
`documentSocketUri`, like the app. Product traffic was never affected —
`main_shell` and `sync_reconcile_controller` already went through it.
Follow-up commit `ab578a90`.

## Desktop

0.13.69 was installed from the published installer, then upgraded on the same
machine by running the 0.14.0 installer manually (user-driven). The installed
binary reports ProductVersion 0.14.0 and launches. The in-app update prompt
path (check → download → run → restart) was not exercised in this round.
