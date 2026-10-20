#Requires -Version 5.1
<#
.SYNOPSIS
  One-time setup of a self-hosted Windows runner, to the baseline the workflows
  assume (what GitHub's windows-latest image ships with). Run once as admin.

.DESCRIPTION
  The workflows are written against GitHub's hosted image, which ships git,
  bash, pwsh, Chocolatey and the MSVC toolchain preinstalled. A self-hosted
  runner has none of that, and installing it is NOT the workflows' job: that
  would fork the YAML between hosted and self-hosted, and re-check on every run.
  So the machine is brought to the same baseline once, here, and the YAML stays
  as it is (2026-10-20, owner's call).

  Idempotent: each item is checked first and skipped when present, so rerunning
  after a partial failure only installs what is still missing.

  Installs:
    - Chocolatey                 (release.yml: `choco install innosetup`)
    - Git for Windows + bash     (actions/checkout, every `shell: bash` step,
                                  dtolnay/rust-toolchain, Flutter itself)
    - PowerShell 7 (pwsh)        (every `shell: pwsh` step)
    - jq, unzip, 7zip            (subosito/flutter-action)
    - Inno Setup 6               (the installer; preinstalled so the release job
                                  does not download it each time)
    - VS 2022 Build Tools, C++ desktop workload + Windows SDK
                                 (Rust's MSVC target and `flutter build windows`)
    - rustup (machine-wide, C:\rust)
                                 (release `windows` + flutter-integration build
                                  the FFI crate with no toolchain step)
    - Long paths (registry + git core.longpaths)
                                 (checkout of this repo exceeds 260 chars)
    - Developer Mode             (Flutter needs symlinks for plugin builds)
  Flutter is NOT installed: the workflows fetch the pinned version (.fvmrc).
  rustup is, but only stable/minimal: rust-toolchain.toml pins the version.

.PARAMETER RestartRunner
  Restart the runner service a minute after finishing, so it picks up the new
  machine PATH. Use it when this script runs INSIDE a runner job (restarting
  synchronously would kill that job mid-step).

.EXAMPLE
  # Elevated PowerShell on the runner machine:
  powershell -ExecutionPolicy Bypass -File scripts\runner-bootstrap-windows.ps1
#>
param([switch]$RestartRunner)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Say($msg) { Write-Host "==> $msg" }

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$admin = ([Security.Principal.WindowsPrincipal]$identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
Say "running as $($identity.Name), admin=$admin"
if (-not $admin) {
  throw ("REFUSED: needs administrator rights to install software. " +
    "Run this from an elevated PowerShell, or (when run by the runner) set the " +
    "runner service to log on as Local System / an admin account and rerun.")
}

# TLS 1.2 for the downloads below; PowerShell 5 does not default to it.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072

function Refresh-Path {
  $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
  $user = [Environment]::GetEnvironmentVariable('Path', 'User')
  $env:Path = "$machine;$user"
}

# --- Chocolatey -------------------------------------------------------------
# By path, not just `Get-Command`: a runner service that has not restarted yet
# still has the old PATH, and the second run re-ran the installer over a
# perfectly good install.
$chocoExe = Join-Path $env:ProgramData 'chocolatey\bin\choco.exe'
if ((Get-Command choco -ErrorAction SilentlyContinue) -or (Test-Path $chocoExe)) {
  if (-not (Get-Command choco -ErrorAction SilentlyContinue)) { $env:Path += ";$(Split-Path $chocoExe)" }
  Say "chocolatey: present"
} else {
  Say "chocolatey: installing"
  Set-ExecutionPolicy Bypass -Scope Process -Force
  Invoke-Expression ((New-Object Net.WebClient).DownloadString('https://community.chocolatey.org/install.ps1'))
  Refresh-Path
}

function Ensure-Choco($name, $present, $extra = @()) {
  if (& $present) { Say "${name}: present"; return }
  Say "${name}: installing"
  & choco install $name -y --no-progress @extra
  if ($LASTEXITCODE -notin 0, 1641, 3010) { throw "choco install $name failed (exit $LASTEXITCODE)" }
  Refresh-Path
}

# Git's bash and unix tools on PATH, the way the hosted image has them: the
# runner resolves `shell: bash` by looking `bash` up on PATH.
Ensure-Choco 'git' { Test-Path "$env:ProgramFiles\Git\bin\bash.exe" } `
  @('--params', '"/GitAndUnixToolsOnPath /NoShellIntegration /NoAutoCrlf"')
Ensure-Choco 'powershell-core' { Test-Path "$env:ProgramFiles\PowerShell\7\pwsh.exe" }
# What subosito/flutter-action shells out to (the hosted image has all three):
# jq reads Flutter's release manifest, unzip / 7z unpack the SDK archive. The
# first real release run on this machine died on `jq not found`.
Ensure-Choco 'jq' { Get-Command jq -ErrorAction SilentlyContinue }
Ensure-Choco 'unzip' { Get-Command unzip -ErrorAction SilentlyContinue }
Ensure-Choco '7zip' { Test-Path "$env:ProgramFiles\7-Zip\7z.exe" }

Ensure-Choco 'innosetup' { Test-Path "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" }

# The MSVC toolchain. `--wait` keeps choco from returning while the VS
# installer is still running in the background, which would race the check.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasVc = { (Test-Path $vswhere) -and (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath) }
Ensure-Choco 'visualstudio2022buildtools' $hasVc @(
  '--package-parameters',
  '"--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --passive --wait --locale en-US"'
)
if (-not (& $hasVc)) { throw "VS Build Tools installed, but the C++ toolset is still missing" }

# --- Rust (rustup) ------------------------------------------------------------
# The hosted image ships rustup, and two Windows jobs lean on that: release.yml's
# `windows` and flutter-integration's `integration` build the FFI crate with no
# toolchain step of their own. Installed machine-wide under C:\rust (the runner
# service may run as Local System, whose profile is no place for a toolchain).
# Only stable/minimal here: rust-toolchain.toml pins the real version, which
# rustup fetches on the first cargo call.
$rustRoot = Join-Path $env:SystemDrive 'rust'
$env:RUSTUP_HOME = Join-Path $rustRoot 'rustup'
$env:CARGO_HOME = Join-Path $rustRoot 'cargo'
$cargoBin = Join-Path $env:CARGO_HOME 'bin'
if (Test-Path (Join-Path $cargoBin 'rustup.exe')) {
  Say "rustup: present"
} else {
  Say "rustup: installing to $rustRoot"
  $init = Join-Path $env:TEMP 'rustup-init.exe'
  Invoke-WebRequest -UseBasicParsing -OutFile $init -Uri 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe'
  & $init -y --no-modify-path --profile minimal --default-toolchain stable
  if ($LASTEXITCODE) { throw "rustup-init failed (exit $LASTEXITCODE)" }
}
[Environment]::SetEnvironmentVariable('RUSTUP_HOME', $env:RUSTUP_HOME, 'Machine')
[Environment]::SetEnvironmentVariable('CARGO_HOME', $env:CARGO_HOME, 'Machine')
$machinePath = [Environment]::GetEnvironmentVariable('Path', 'Machine')
if (($machinePath -split ';') -notcontains $cargoBin) {
  [Environment]::SetEnvironmentVariable('Path', "$machinePath;$cargoBin", 'Machine')
}
Refresh-Path

# --- Long paths ----------------------------------------------------------------
# The hosted image has them on; without them the first checkout of this repo on
# a bare runner died extracting the archive (a path past 260 chars).
$fs = 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem'
if ((Get-ItemProperty -Path $fs -Name LongPathsEnabled -ErrorAction SilentlyContinue).LongPathsEnabled -eq 1) {
  Say "long paths: on"
} else {
  Say "long paths: enabling"
  Set-ItemProperty -Path $fs -Name LongPathsEnabled -Value 1 -Type DWord
}
& git config --system core.longpaths true

# --- Developer Mode (symlinks for Flutter plugin builds) --------------------
$dev = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock'
if ((Get-ItemProperty -Path $dev -Name AllowDevelopmentWithoutDevLicense -ErrorAction SilentlyContinue).AllowDevelopmentWithoutDevLicense -eq 1) {
  Say "developer mode: on"
} else {
  Say "developer mode: enabling"
  New-Item -Path $dev -Force | Out-Null
  Set-ItemProperty -Path $dev -Name AllowDevelopmentWithoutDevLicense -Value 1 -Type DWord
}

# --- Report -----------------------------------------------------------------
Refresh-Path
Say "result:"
foreach ($t in 'git', 'bash', 'pwsh', 'choco', 'iscc', 'rustup', 'cargo', 'jq', 'unzip', '7z') {
  $c = Get-Command $t -ErrorAction SilentlyContinue
  if (-not $c -and $t -eq 'iscc') { $c = Get-Item "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe" -ErrorAction SilentlyContinue }
  '    {0,-6} {1}' -f $t, $(if ($c) { if ($c.Source) { $c.Source } else { $c.FullName } } else { 'MISSING' })
}
'    msvc   ' + (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)

# The runner service read PATH when it started; new tools are invisible to its
# jobs until it restarts.
$svc = Get-Service -Name 'actions.runner.*' -ErrorAction SilentlyContinue
if ($RestartRunner -and $svc) {
  $names = ($svc | ForEach-Object { $_.Name }) -join ','
  Say "restarting runner service(s) in ~60s: $names"
  # Scheduled-task cmdlets, not schtasks.exe: Windows PowerShell 5 mangles the
  # embedded quotes when passing a /TR command line to a native exe (first run,
  # 2026-10-20: "Invalid argument/option - '-Name'", and the restart never
  # happened). The cmdlets take the arguments as data, no command line to split.
  $quoted = ($svc | ForEach-Object { "'$($_.Name)'" }) -join ','
  $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -Command Restart-Service -Name $quoted -Force"
  $trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddSeconds(60)
  Register-ScheduledTask -TaskName 'mica-runner-restart' -Action $action -Trigger $trigger `
    -User 'SYSTEM' -RunLevel Highest -Force | Out-Null
} elseif ($svc) {
  Say "done. Restart the runner service so its jobs see the new PATH: Restart-Service '$($svc[0].Name)'"
} else {
  Say "done. (No runner service found; if the runner runs interactively, restart run.cmd.)"
}
