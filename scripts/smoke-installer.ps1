# Silent-install the packaged installer and prove the installed app survives
# its first frame within 30 s, then survives another 10 s. Lifted from the
# `windows` job of .github/workflows/release.yml so the
# LOCAL release train runs the same check: the in-app updater ships this exact
# .exe to every user, and build success only proves it compiled.
# Close an existing Mica client first: its single-instance lock blocks the probe.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-installer.ps1 <setup.exe>
param([Parameter(Mandatory = $true)][string]$Setup)
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

$installDir = Join-Path $env:TEMP ("mica-smoke-" + [guid]::NewGuid().ToString('N'))
Write-Host "Silent-installing $Setup -> $installDir"
$inst = $null
$app = $null
try {
  $inst = Start-Process -FilePath $Setup `
    -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SKIPRUN', "/DIR=$installDir" `
    -WindowStyle Hidden -PassThru
  $exe = Join-Path $installDir 'mica_flutter.exe'
  Wait-MicaInstaller -Installer $inst -Executable $exe
  Write-Host "Installed OK: $exe"

  $probe = Join-Path $installDir 'first-frame.marker'
  $previousProbe = $env:MICA_STARTUP_PROBE_FILE
  try {
    $env:MICA_STARTUP_PROBE_FILE = $probe
    $app = Start-Process -FilePath $exe -WorkingDirectory $installDir -WindowStyle Hidden -PassThru
  } finally {
    $env:MICA_STARTUP_PROBE_FILE = $previousProbe
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
    throw "Mica did not produce its first Flutter frame within 30s"
  }
  Start-Sleep -Seconds 10
  $app.Refresh()
  if ($app.HasExited) {
    Write-MicaLoaderDiagnostics -Directory $installDir -ExitCode $app.ExitCode
    throw "Mica exited within 10s after its first frame (exit code 0x$('{0:X8}' -f $app.ExitCode)) - install/launch smoke FAILED"
  }
  Write-Host "Mica rendered its first frame and stayed alive for 10s (pid $($app.Id)) - smoke OK"
} finally {
  if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
  if ($inst -and -not $inst.HasExited) { Stop-Process -Id $inst.Id -Force -ErrorAction SilentlyContinue }
  $uninstaller = Join-Path $installDir 'unins000.exe'
  if (Test-Path $uninstaller) {
    Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -Wait
  }
}
