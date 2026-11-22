# Silent-install the packaged installer and prove the installed app survives
# its first frame within 30 s, then survives another 10 s. Lifted from the
# `windows` job of .github/workflows/release.yml so the
# LOCAL release train runs the same check: the in-app updater ships this exact
# .exe to every user, and build success only proves it compiled.
#
# The local release train runs this on a machine someone uses every day, often
# with their own Mica open, so the probe must not touch that install or its data
# (#179):
#   - /MICASMOKE makes the installer skip the Start-menu/desktop shortcuts and
#     the uninstall registry entry, which share their paths with the real
#     install (it also implies /SKIPRUN: this script is the only launcher).
#   - The app runs with APPDATA / LOCALAPPDATA / TEMP / TMP pointed at a fresh
#     profile under $root, so it never opens the real %APPDATA%\mica store.db,
#     and the runner's single-instance lock (scoped to the data directory)
#     does not collide with the running Mica.
#   - Everything lives under one $root that is removed at the end.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-installer.ps1 <setup.exe>
param(
  [Parameter(Mandatory = $true)][string]$Setup,
  [string]$ExpectedVersion = ''
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'wait-installer.ps1')

function Write-MicaLoaderDiagnostics {
  param([string]$Directory, [int]$ExitCode)
  if ($ExitCode -ne [int]0xC0000135) { return }
  try {
    $vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
    if (-not (Test-Path -LiteralPath $vswhere)) { return }
    $vs = & $vswhere -latest -products * -property installationPath
    $dumpbin = Get-ChildItem "$vs\VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe" | Select-Object -Last 1
    if (-not $dumpbin) { return }
    Get-ChildItem $Directory -Recurse -Include *.exe,*.dll | ForEach-Object {
      $binary = $_
      & $dumpbin.FullName /nologo /dependents $binary.FullName |
        ForEach-Object { $_.Trim() } | Where-Object { $_ -match '^[\w.-]+\.dll$' } |
        Where-Object { $_ -notmatch '^(api|ext)-ms-' } |
        Where-Object { -not (Test-Path (Join-Path $Directory $_)) -and -not (Test-Path (Join-Path "$env:SystemRoot\System32" $_)) } |
        ForEach-Object { Write-Host "MISSING: $($binary.Name) imports $_" }
    }
  } catch {
    Write-Warning "Could not inspect loader dependencies: $_"
  }
}

$root = Join-Path $env:TEMP ("mica-smoke-" + [guid]::NewGuid().ToString('N'))
$installDir = Join-Path $root 'app'
$profileDir = Join-Path $root 'profile'
Write-Host "Silent-installing $Setup -> $installDir"
$inst = $null
$app = $null
try {
  $inst = Start-Process -FilePath $Setup `
    -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SKIPRUN', '/MICASMOKE', "/DIR=$installDir" `
    -WindowStyle Hidden -PassThru
  $exe = Join-Path $installDir 'mica-rust.exe'
  Wait-MicaInstaller -Installer $inst -Executable $exe
  Write-Host "Installed OK: $exe"
  if ($ExpectedVersion) {
    $actualVersion = ((Get-Item -LiteralPath $exe).VersionInfo.ProductVersion -split '\+')[0]
    if ($actualVersion -ne $ExpectedVersion) {
      throw "Installed app version '$actualVersion' does not match release '$ExpectedVersion'"
    }
    Write-Host "Installed product version: $actualVersion"
  }

  $probe = Join-Path $root 'first-frame.marker'
  $isolated = [ordered]@{
    APPDATA = Join-Path $profileDir 'roaming'
    LOCALAPPDATA = Join-Path $profileDir 'local'
    TEMP = Join-Path $profileDir 'temp'
    TMP = Join-Path $profileDir 'temp'
    MICA_STARTUP_PROBE_FILE = $probe
  }
  $previous = @{}
  foreach ($name in $isolated.Keys) {
    $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
  }
  foreach ($dir in @($isolated.APPDATA, $isolated.LOCALAPPDATA, $isolated.TEMP)) {
    New-Item -ItemType Directory -Force $dir | Out-Null
  }
  # Only the app sees the isolated environment: the installer and uninstaller
  # above and below run with the caller's own.
  try {
    foreach ($name in $isolated.Keys) {
      [Environment]::SetEnvironmentVariable($name, $isolated[$name], 'Process')
    }
    $app = Start-Process -FilePath $exe -WorkingDirectory $installDir -WindowStyle Hidden -PassThru
  } finally {
    foreach ($name in $isolated.Keys) {
      [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
    }
  }
  $firstFrameDeadline = (Get-Date).AddSeconds(30)
  while (-not (Test-Path -LiteralPath $probe) -and (Get-Date) -lt $firstFrameDeadline) {
    $app.Refresh()
    if ($app.HasExited) {
      Write-MicaLoaderDiagnostics -Directory $installDir -ExitCode $app.ExitCode
      throw "Mica exited before its first frame (exit code 0x$('{0:X8}' -f $app.ExitCode))"
    }
    Start-Sleep -Milliseconds 250
  }
  if (-not (Test-Path -LiteralPath $probe)) {
    throw "Mica did not produce its first native frame within 30s"
  }
  Start-Sleep -Seconds 10
  $app.Refresh()
  if ($app.HasExited) {
    Write-MicaLoaderDiagnostics -Directory $installDir -ExitCode $app.ExitCode
    throw "Mica exited within 10s after its first frame (exit code 0x$('{0:X8}' -f $app.ExitCode)) - install/launch smoke FAILED"
  }
  # Proves the isolation took effect: the app created its profile under the
  # redirected APPDATA instead of the caller's real one.
  $isolatedProfile = Join-Path $isolated.APPDATA 'mica'
  if (-not (Test-Path -LiteralPath $isolatedProfile)) {
    throw "Mica did not create its profile under the isolated APPDATA ($isolatedProfile)"
  }
  Write-Host "Mica rendered its first frame and stayed alive for 10s (pid $($app.Id)) - smoke OK"
} finally {
  if ($app -and -not $app.HasExited) {
    Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue
    $app.WaitForExit(10000) | Out-Null
  }
  if ($inst -and -not $inst.HasExited) { Stop-Process -Id $inst.Id -Force -ErrorAction SilentlyContinue }
  $uninstaller = Join-Path $installDir 'unins000.exe'
  if (Test-Path $uninstaller) {
    Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -Wait
  }
  Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
