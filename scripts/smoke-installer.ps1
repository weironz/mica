# Silent-install the packaged installer and prove the installed app survives
# its first frame within 30 s, then survives another 10 s. Lifted from the
# `windows` job of .github/workflows/release.yml so the
# LOCAL release train runs the same check: the in-app updater ships this exact
# .exe to every user, and build success only proves it compiled.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-installer.ps1 <setup.exe>
param([Parameter(Mandatory = $true)][string]$Setup)
$ErrorActionPreference = 'Stop'

$installDir = Join-Path $env:TEMP ("mica-smoke-" + [guid]::NewGuid().ToString('N'))
Write-Host "Silent-installing $Setup -> $installDir"
$inst = Start-Process -FilePath $Setup `
  -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SKIPRUN', "/DIR=$installDir" `
  -PassThru
$exe = Join-Path $installDir 'mica_flutter.exe'
$deadline = (Get-Date).AddSeconds(120)
while (-not (Test-Path $exe) -and (Get-Date) -lt $deadline) {
  if ($inst.HasExited -and $inst.ExitCode -ne 0) {
    throw "installer exited with code $($inst.ExitCode) before producing $exe"
  }
  Start-Sleep -Seconds 2
}
if (-not (Test-Path $exe)) { throw "installed exe not found after 120s: $exe" }
Write-Host "Installed OK: $exe"

$probe = Join-Path $installDir 'first-frame.marker'
$previousProbe = $env:MICA_STARTUP_PROBE_FILE
try {
  $env:MICA_STARTUP_PROBE_FILE = $probe
  $app = Start-Process -FilePath $exe -WorkingDirectory $installDir -PassThru
} finally {
  $env:MICA_STARTUP_PROBE_FILE = $previousProbe
}
try {
  $firstFrameDeadline = (Get-Date).AddSeconds(30)
  while (-not (Test-Path -LiteralPath $probe) -and (Get-Date) -lt $firstFrameDeadline) {
    $app.Refresh()
    if ($app.HasExited) {
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
    throw "Mica exited within 10s after its first frame (exit code 0x$('{0:X8}' -f $app.ExitCode)) - install/launch smoke FAILED"
  }
  Write-Host "Mica rendered its first frame and stayed alive for 10s (pid $($app.Id)) - smoke OK"
} finally {
  if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force -ErrorAction SilentlyContinue }
  if ($inst -and -not $inst.HasExited) { Stop-Process -Id $inst.Id -Force -ErrorAction SilentlyContinue }
  $uninstaller = Join-Path $installDir 'unins000.exe'
  if (Test-Path $uninstaller) {
    Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait
  }
}
