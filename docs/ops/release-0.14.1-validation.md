# 0.14.1 deployment and acceptance

Date: 2026-10-30. Release tag `v0.14.1` points to `e0a2ec39`. Patch release:
the two fixes that landed on main after 0.14.0, no SQL migrations.

- `66f9f796` nginx: `Cache-Control: no-cache` on `/mica_md/` (the wasm engine
  and its JS glue) and `/yjs_bundle.js`; a missing file under `/mica_md/`
  returns 404 instead of falling back to `index.html`. Both fixed-name entry
  files now refuse stale caching like the rest.
- `ab578a90` the browser sync self-test probe announces the sync protocol
  version (`v=1`) via `documentSocketUri`, like the app. Test-hook only;
  product traffic was never affected.

## Gates and artifacts

- `just ship 0.14.1`: the first run failed at the installer smoke — a live
  0.14.0 instance held the single-instance mutex, so the smoke-launched
  binary exited with code 0 before its first frame. Closing the running
  Mica and rerunning `just ship 0.14.1` resumed from the tag (gate already
  green) and completed: `SHIP_EXIT=0`.
- Gate receipts (first run): rust suites all green, wire gold 9 passed,
  wasm `mica_md` 203,116 B gzipped against the 262,144 B budget, Flutter
  analyze 0/0 and unit tests within floor, installer smoke OK (second run).
- Published assets: `Mica-Setup-0.14.1.exe`, `mica-cli-0.14.1-windows-x64.exe`,
  `mica-cli-0.14.1-linux-x64`, `sha256-windows.txt`, `sha256-linux.txt`.
  Release public; all three images pushed to ACR and Docker Hub.

## Deployment

- Rehearsal (`--check --diff`): 35 ok, 0 failed; changes limited to `.env`
  render, image pull (web image carries the nginx fix), backup sidecar.
  Compose unchanged; no migrations ("no migrations between v0.14.0 and
  v0.14.1 — no restore point needed"); shared Traefik untouched.
- `just deploy-prod 0.14.1`: `ok=35 changed=4 failed=0`; in-play verification
  reported `{"status":"ready","service":"mica-api-server","version":"0.14.1"}`,
  `index: 200`, bundle digest `9821d6008017…`.

## Verification

Production:

- `GET /api/health` → `{"status":"ok","service":"mica-api-server","version":"0.14.1"}`.
- `/mica_md/mica_md_bg.wasm` → 200, `Cache-Control: no-cache`,
  `Content-Type: application/wasm`.
- `/mica_md/mica_md.js` → 200, `Cache-Control: no-cache`.
- `/yjs_bundle.js` → 200, `Cache-Control: no-cache`.
- `/mica_md/missing.wasm` → **404** (previously 200 with index.html).
- The production login page renders with footer v0.14.1; the engine loads
  (`micaMdReady`, no `micaMdError`).

Local dev stack:

- `node e2e/web_e2e.mjs` → **17/17 ok**, including "two web sessions converge
  through the real server" and "the edit made in session A arrived in session B"
  — the exact path `ab578a90` fixed (before it, the probe was refused with
  `client_too_old`). The local nginx also serves `no-cache` on the two engine
  files and the yjs bundle (assertions in the same run).
- Note: the release gate's DB tests reset the dev database (demo account gone);
  `just seed-dev` restored it before the e2e run.

Desktop (Windows, previously on 0.14.0):

- In-app update path exercised end to end for the first time: the sidebar
  "新版本" badge appeared after release publish, the confirm dialog offered
  0.14.1, and one click on 更新并重启 downloaded, closed the app, ran the
  installer and relaunched.
- After update: installed `mica_flutter.exe` reports ProductVersion 0.14.1
  and the new process runs. This closes the gap left in the 0.14.0 round
  (that upgrade was a manually launched installer).

## Independent production check (reviewer)

Run from a separate Linux environment after the deploy, without credentials:

- `scripts/verify-prod.sh 0.14.1` → exit 0; `/api/health` reports 0.14.1.
- Both fixes are in the tag (`66f9f796`, `ab578a90` are ancestors of `e0a2ec39`).
- Latest release is v0.14.1 with 5 assets; `Mica-Setup-0.14.1.exe`
  (41,450,120 B) matches `sha256-windows.txt`.
- `/mica_md/mica_md.js`, `/mica_md/mica_md_bg.wasm`, `/yjs_bundle.js` →
  `Cache-Control: no-cache`; wasm is `application/wasm`; `/mica_md/missing.wasm`
  → 404; a repeat request with `If-None-Match` → 304 with an empty body, so
  `no-cache` costs a revalidation, not a re-download.
- Headless Chromium: login page renders with footer v0.14.1, both engine files
  load with `no-cache`, `micaMdReady` resolves without `micaMdError`,
  `engineInfo` reports schema 1, and a parse → serialize round trip of a
  heading, inline marks, a link and a table matches the canonical output.
  No console errors, no failed requests.
