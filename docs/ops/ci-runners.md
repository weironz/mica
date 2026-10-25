# CI runner notes

This repo runs Actions almost exclusively on self-hosted runners
(2026-10-20: the hosted quota ran out, so push/PR triggers were switched to
`workflow_dispatch`). Runner inventory:

- `home-workstation-linux-mica-01` — label pair `[self-hosted, Linux]`
- `home-workstation-windows-mica-01` — label pair `[self-hosted, Windows]`

## Pre-installed Flutter on self-hosted runners

The action reuses a matching SDK in `FLUTTER_ROOT`, or checks
`RUNNER_TOOL_CACHE/flutter/<version>/stable` when `FLUTTER_ROOT` is unset.
To pre-install the exact version after a `.fvmrc` bump, run as the runner
service account:

```powershell
$version = (Get-Content .fvmrc | ConvertFrom-Json).flutter
$sdkPath = "$env:RUNNER_TOOL_CACHE\flutter\$version\stable"
git clone --depth 1 --branch $version https://github.com/flutter/flutter.git $sdkPath
git -C $sdkPath switch -c stable
& "$sdkPath\bin\flutter.bat" precache --web --windows
& "$sdkPath\bin\flutter.bat" --version
```

`RUNNER_TOOL_CACHE` points at the runner service account's `_work/_tool`
directory. Use the runner job's value, rather than the interactive user's
home directory. The [setup-flutter composite
action](../../.github/actions/setup-flutter/action.yml) checks that location
first and skips the SDK download entirely when the version and channel match;
a stale or missing install falls back to subosito/flutter-action@v2, which
then downloads the exact pinned version as before. On hosted runners the
pre-install check is skipped unconditionally.

Set `MICA_SKIP_FLUTTER_SETUP=true` on a job to bypass the shortcut and force a
full toolchain setup (use it when debugging the toolchain itself).

## Local checks

`just ci` runs structure, Rust format/build/Clippy, database tests, Flutter
analysis, the complete unit/widget suite, the release web build and font checks.
It also tests the CI job-selection rules. `just ci rust` and `just ci flutter`
select the corresponding local half without changing any gate thresholds.

Actions remains manual. `CI` defaults to `scope=full`, which runs every job.
`scope=auto` selects jobs from `base_ref..HEAD` (default `HEAD^..HEAD`);
unknown paths select every job. Use an explicit `base_ref` to validate several
commits together. `Flutter Integration Tests` defaults to `suite=full`;
`suite=smoke` runs two offline files and the live cloud-sync file.

Native Windows/FFI integration remains a separate local command:
`just test-integration`. A green `just ci` does not include that command, the
real-stack browser suite, container startup or the MCP inspector.
