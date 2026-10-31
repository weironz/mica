# 0.14.2 deployment and acceptance

Date: 2026-10-31. Release tag `v0.14.2` points to the `chore(release): 0.14.2`
commit. Patch release over the main commits since 0.14.1 (tabs keyboard and
narrow-shell switcher with restore-after-restart, bar tooltips back in the
canvas-following bars and off under semantics, the export title fix for a
same-named heading, dropping the unused BlockListItem widget, roadmap archive
bookkeeping). No SQL migrations.

## Gates and artifacts

- `just ship 0.14.2` passed end to end on the first run: `SHIP_EXIT=0`.
  The running desktop client was closed before starting, so the installer
  smoke (single-instance mutex) passed without a retry this time.
- Gate receipts: rust suites all green (`receipt: 1160 passed`, floor 500);
  wire gold `receipt: 9 passed`; wasm `mica_md` 203,116 B gzipped against the
  262,144 B budget; Flutter unit tests `receipt: 2074 passed` (floor 1000);
  Flutter analyze 0 error / 0 warning; web bundle built.
- Published assets: `Mica-Setup-0.14.2.exe`, `mica-cli-0.14.2-windows-x64.exe`,
  `mica-cli-0.14.2-linux-x64`, `sha256-windows.txt`, `sha256-linux.txt`.
  Release public; all three images pushed to ACR and Docker Hub.

## Deployment

- Rehearsal (`--check --diff`): 35 ok, 0 failed; changes limited to the
  version-dependent pieces (`.env` render, image pull, backup sidecar).
  Compose unchanged; "no migrations between v0.14.1 and v0.14.2 — no restore
  point needed"; shared Traefik untouched.
- `just deploy-prod 0.14.2`: `ok=35 changed=4 failed=0`; in-play verification
  reported `{"status":"ready","service":"mica-api-server","version":"0.14.2"}`,
  `index: 200`, bundle digest `09aac965be9a…`.

## Verification

Production:

- `GET /api/health` → `{"status":"ok","service":"mica-api-server","version":"0.14.2"}`.
- `/mica_md/mica_md_bg.wasm` → 200, `Cache-Control: no-cache`,
  `Content-Type: application/wasm`.
- `/mica_md/mica_md.js` → 200, `Cache-Control: no-cache`.
- `/yjs_bundle.js` → 200, `Cache-Control: no-cache`.
- `/mica_md/missing.wasm` → 404.
- The production login page renders with footer v0.14.2; the engine loads
  (`micaMdReady`, no `micaMdError`).

Local dev stack (rebuilt by the ship; the dev API container was restarted via
`just dev` and the demo account was still present):

- `node e2e/web_e2e.mjs` → **17/17 ok**, including "two web sessions converge
  through the real server" and "the edit made in session A arrived in
  session B", plus the no-cache assertions on the two engine files and the
  yjs bundle.

## Desktop

In-app update 0.14.1 → 0.14.2 is user-driven this round (the machine was in
active use, so GUI automation could not take the foreground without
interrupting it). Outcome is recorded below once confirmed.
