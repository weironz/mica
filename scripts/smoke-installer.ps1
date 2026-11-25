# Silent-install the packaged installer and prove the installed app survives
# its first frame within 30 s, then survives another 10 s. Lifted from the
# `windows` job of .github/workflows/release.yml so the
# LOCAL release train runs the same check: the in-app updater ships this exact
# .exe to every user, and build success only proves it compiled.
#
# Optional `-UpgradeFrom <prev-setup.exe>` runs a full in-place upgrade smoke
# in the same isolated directory & profile: installs the previous release,
# boots it once to initialize `%APPDATA%\mica`, writes an upgrade sentinel,
# shuts it down cleanly, overwrites it in place with `$Setup`, verifies the
# upgraded version & 10 s survival, and proves user profile data survived.
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
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/smoke-installer.ps1 -Setup <new.exe> -ExpectedVersion 0.15.1 -UpgradeFrom <prev.exe> -UpgradeFromVersion 0.15.0
param(
  [Parameter(Mandatory = $true)][string]$Setup,
  [string]$ExpectedVersion = '',
  [string]$UpgradeFrom = '',
  [string]$UpgradeFromVersion = ''
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

function Resolve-InstalledMicaExe {
  param([string]$Directory)
  foreach ($candidate in @('mica-rust.exe', 'mica_flutter.exe', 'mica.exe')) {
    $path = Join-Path $Directory $candidate
    if (Test-Path -LiteralPath $path) { return $path }
  }
  return (Join-Path $Directory 'mica-rust.exe')
}

function Assert-InstalledVersion {
  param(
    [string]$Executable,
    [string]$Expected,
    [string]$Label = 'Installed',
    [string]$FallbackSetupPath = ''
  )
  if (-not $Expected) { return }
  $actualVersion = ((Get-Item -LiteralPath $Executable).VersionInfo.ProductVersion -split '\+')[0]
  if (-not $actualVersion -and $FallbackSetupPath) {
    # Legacy v0.15.0 pure-Rust binary predated VS_VERSION_INFO embedding; accept
    # only when the verified baseline installer's version metadata or filename
    # matches the expected baseline release.
    $vi = (Get-Item -LiteralPath $FallbackSetupPath).VersionInfo
    $candidates = @($vi.ProductVersion, $vi.FileVersion) | Where-Object { $_ }
    $matched = $false
    foreach ($c in $candidates) {
      $clean = ($c -split '\+')[0].Trim()
      $prefix = ($clean -split '\.')[0..([Math]::Min(2, ($clean -split '\.').Length - 1))] -join '.'
      if ($clean -eq $Expected -or $prefix -eq $Expected) {
        $matched = $true
        break
      }
    }
    if (-not $matched -and (Split-Path -Leaf $FallbackSetupPath) -eq "Mica-Setup-$Expected.exe") {
      $matched = $true
    }
    if ($matched) {
      Write-Host "$Label executable ($Expected) has no embedded ProductVersion; verified via baseline installer $(Split-Path -Leaf $FallbackSetupPath)"
      return
    }
  }
  if ($actualVersion -ne $Expected) {
    throw "$Label app version '$actualVersion' does not match expected '$Expected' ($Executable)"
  }
  Write-Host "$Label product version: $actualVersion"
}

function Invoke-MicaIsolatedLaunch {
  param(
    [string]$Executable,
    [string]$InstallDir,
    [System.Collections.IDictionary]$IsolatedEnv,
    [string]$ProbePath,
    [int]$SurviveSeconds = 10,
    [string]$Label = 'Mica',
    [switch]$AllowLegacyProfileReady
  )
  if (Test-Path -LiteralPath $ProbePath) {
    Remove-Item -LiteralPath $ProbePath -Force
  }
  $previous = @{}
  foreach ($name in $IsolatedEnv.Keys) {
    $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
  }
  $proc = $null
  try {
    try {
      foreach ($name in $IsolatedEnv.Keys) {
        [Environment]::SetEnvironmentVariable($name, $IsolatedEnv[$name], 'Process')
      }
      $proc = Start-Process -FilePath $Executable -WorkingDirectory $InstallDir -WindowStyle Hidden -PassThru
    } finally {
      foreach ($name in $IsolatedEnv.Keys) {
        [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
      }
    }

    $isolatedProfile = Join-Path $IsolatedEnv.APPDATA 'mica'
    $legacyStoreDb = Join-Path (Join-Path $isolatedProfile 'local') 'store.db'
    $flatStoreDb = Join-Path $isolatedProfile 'store.db'
    $readyByLegacyProfile = $false
    $firstFrameDeadline = (Get-Date).AddSeconds(30)
    while (-not (Test-Path -LiteralPath $ProbePath) -and (Get-Date) -lt $firstFrameDeadline) {
      $proc.Refresh()
      if ($proc.HasExited) {
        Write-MicaLoaderDiagnostics -Directory $InstallDir -ExitCode $proc.ExitCode
        throw "$Label exited before its first frame (exit code 0x$('{0:X8}' -f $proc.ExitCode))"
      }
      if ($AllowLegacyProfileReady -and ((Test-Path -LiteralPath $legacyStoreDb) -or (Test-Path -LiteralPath $flatStoreDb))) {
        $readyByLegacyProfile = $true
        break
      }
      Start-Sleep -Milliseconds 250
    }
    if (-not (Test-Path -LiteralPath $ProbePath) -and -not $readyByLegacyProfile) {
      throw "$Label did not produce its first native frame within 30s"
    }
    if ($SurviveSeconds -gt 0) {
      Start-Sleep -Seconds $SurviveSeconds
      $proc.Refresh()
      if ($proc.HasExited) {
        Write-MicaLoaderDiagnostics -Directory $InstallDir -ExitCode $proc.ExitCode
        throw "$Label exited within ${SurviveSeconds}s after its first frame (exit code 0x$('{0:X8}' -f $proc.ExitCode)) - install/launch smoke FAILED"
      }
    }
    # Proves the isolation took effect: the app created its profile under the
    # redirected APPDATA instead of the caller's real one.
    if (-not (Test-Path -LiteralPath $isolatedProfile)) {
      throw "$Label did not create its profile under the isolated APPDATA ($isolatedProfile)"
    }
    if ($readyByLegacyProfile) {
      Write-Host "$Label initialized isolated profile store.db and stayed alive for ${SurviveSeconds}s (pid $($proc.Id)) - baseline OK"
    } else {
      Write-Host "$Label rendered its first frame and stayed alive for ${SurviveSeconds}s (pid $($proc.Id)) - OK"
    }
  } finally {
    if ($proc -and -not $proc.HasExited) {
      Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
      $proc.WaitForExit(10000) | Out-Null
    }
  }
}

$root = Join-Path $env:TEMP ("mica-smoke-" + [guid]::NewGuid().ToString('N'))
$installDir = Join-Path $root 'app'
$profileDir = Join-Path $root 'profile'
$probe = Join-Path $root 'first-frame.marker'
$isolated = [ordered]@{
  APPDATA = Join-Path $profileDir 'roaming'
  LOCALAPPDATA = Join-Path $profileDir 'local'
  TEMP = Join-Path $profileDir 'temp'
  TMP = Join-Path $profileDir 'temp'
  MICA_STARTUP_PROBE_FILE = $probe
}
foreach ($dir in @($isolated.APPDATA, $isolated.LOCALAPPDATA, $isolated.TEMP)) {
  New-Item -ItemType Directory -Force $dir | Out-Null
}

$inst = $null
try {
  $upgradeSentinelPath = $null
  $upgradeSentinelValue = $null
  $prevStoreDbExisted = $false

  if ($UpgradeFrom) {
    Write-Host "Upgrade baseline: silent-installing $UpgradeFrom -> $installDir"
    $inst = Start-Process -FilePath $UpgradeFrom `
      -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SKIPRUN', '/MICASMOKE', "/DIR=$installDir" `
      -WindowStyle Hidden -PassThru
    $inst.WaitForExit(180000) | Out-Null
    if (-not $inst.HasExited) {
      throw "Baseline installer did not finish within 180s: $UpgradeFrom"
    }
    if ($inst.ExitCode -ne 0) {
      throw "Baseline installer exited with code $($inst.ExitCode): $UpgradeFrom"
    }
    $prevExe = Resolve-InstalledMicaExe -Directory $installDir
    if (-not (Test-Path -LiteralPath $prevExe)) {
      throw "Baseline executable not found after install in $installDir"
    }
    Write-Host "Baseline installed OK: $prevExe"
    Assert-InstalledVersion -Executable $prevExe -Expected $UpgradeFromVersion -Label 'Baseline' -FallbackSetupPath $UpgradeFrom

    $legacyBaseline = ($UpgradeFromVersion -eq '0.15.0')
    $baselineEnv = [ordered]@{}
    foreach ($k in $isolated.Keys) { $baselineEnv[$k] = $isolated[$k] }
    if ($legacyBaseline) {
      # Legacy v0.15.0 predated honoring %APPDATA% in VaultStore::open_default(),
      # so pin MICA_DATA_DIR to the canonical isolated `mica\local` path while
      # leaving the upgraded candidate strictly on %APPDATA% alone.
      $baselineEnv['MICA_DATA_DIR'] = Join-Path (Join-Path $isolated.APPDATA 'mica') 'local'
    }
    Invoke-MicaIsolatedLaunch -Executable $prevExe -InstallDir $installDir `
      -IsolatedEnv $baselineEnv -ProbePath $probe -SurviveSeconds 2 -Label 'Baseline Mica' `
      -AllowLegacyProfileReady:$legacyBaseline

    $isolatedProfile = Join-Path $isolated.APPDATA 'mica'
    $upgradeSentinelPath = Join-Path $isolatedProfile 'upgrade-sentinel.txt'
    $upgradeSentinelValue = "mica-upgrade-sentinel-" + [guid]::NewGuid().ToString('N')
    Set-Content -LiteralPath $upgradeSentinelPath -Value $upgradeSentinelValue -NoNewline
    $prevStoreDb = Join-Path (Join-Path $isolatedProfile 'local') 'store.db'
    if (-not (Test-Path -LiteralPath $prevStoreDb)) {
      $prevStoreDb = Join-Path $isolatedProfile 'store.db'
    }
    $prevStoreDbExisted = Test-Path -LiteralPath $prevStoreDb
    $prevStoreDbRel = if ($prevStoreDbExisted) { $prevStoreDb.Substring($isolatedProfile.Length).TrimStart('\') } else { '' }
    Write-Host "Baseline profile initialized (store.db=$prevStoreDbExisted); starting in-place upgrade with $Setup"
  }

  Write-Host "Silent-installing $Setup -> $installDir"
  $inst = Start-Process -FilePath $Setup `
    -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SKIPRUN', '/MICASMOKE', "/DIR=$installDir" `
    -WindowStyle Hidden -PassThru
  $exe = Join-Path $installDir 'mica-rust.exe'
  Wait-MicaInstaller -Installer $inst -Executable $exe
  Write-Host "Installed OK: $exe"
  Assert-InstalledVersion -Executable $exe -Expected $ExpectedVersion -Label 'Installed'

  Invoke-MicaIsolatedLaunch -Executable $exe -InstallDir $installDir `
    -IsolatedEnv $isolated -ProbePath $probe -SurviveSeconds 10 -Label 'Mica'

  if ($upgradeSentinelPath) {
    if (-not (Test-Path -LiteralPath $upgradeSentinelPath)) {
      throw "In-place upgrade removed isolated profile sentinel: $upgradeSentinelPath"
    }
    $afterValue = Get-Content -LiteralPath $upgradeSentinelPath -Raw
    if ($afterValue -ne $upgradeSentinelValue) {
      throw "In-place upgrade corrupted profile sentinel (expected '$upgradeSentinelValue', got '$afterValue')"
    }
    if ($prevStoreDbExisted) {
      $afterStoreDb = Join-Path (Join-Path $isolated.APPDATA 'mica') $prevStoreDbRel
      if (-not (Test-Path -LiteralPath $afterStoreDb) -or (Get-Item -LiteralPath $afterStoreDb).Length -eq 0) {
        throw "In-place upgrade lost or emptied previous store.db: $afterStoreDb"
      }
    }
    Write-Host "In-place upgrade preserved isolated user profile & store.db - upgrade smoke OK"
  }
} finally {
  if ($inst -and -not $inst.HasExited) { Stop-Process -Id $inst.Id -Force -ErrorAction SilentlyContinue }
  $uninstaller = Join-Path $installDir 'unins000.exe'
  if (Test-Path $uninstaller) {
    Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -WindowStyle Hidden -Wait
  }
  Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
