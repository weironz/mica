$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'wait-installer.ps1')
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$fixture = Join-Path $tempRoot ('mica-installer-wait-test-' + [guid]::NewGuid().ToString('N'))
$fixture = [IO.Path]::GetFullPath($fixture)
if (-not $fixture.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) {
  throw 'Test directory is outside the temporary workspace'
}
New-Item -ItemType Directory -Path $fixture | Out-Null
$fakeScript = Join-Path $fixture 'fake-installer.ps1'
$fakeSource = @'
param([string]$Directory, [int]$DelayMilliseconds, [int]$Code, [switch]$NoExecutable)
New-Item -ItemType Directory -Path $Directory -Force | Out-Null
if (-not $NoExecutable) { Set-Content -LiteralPath (Join-Path $Directory 'mica_flutter.exe') -Value 'fixture only' }
Start-Sleep -Milliseconds $DelayMilliseconds
Set-Content -LiteralPath (Join-Path $Directory 'dependency.dll') -Value 'fixture only'
exit $Code
'@
[IO.File]::WriteAllText($fakeScript, $fakeSource)
$processes = New-Object System.Collections.Generic.List[System.Diagnostics.Process]

function Start-InstallerFixture {
  param([string]$Name, [int]$Delay, [int]$Code, [switch]$NoExecutable)
  $directory = Join-Path $fixture $Name
  $processArgs = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$fakeScript`"",
    '-Directory', "`"$directory`"", '-DelayMilliseconds', $Delay, '-Code', $Code)
  if ($NoExecutable) { $processArgs += '-NoExecutable' }
  $process = Start-Process -FilePath powershell.exe -ArgumentList $processArgs -WindowStyle Hidden -PassThru
  $processes.Add($process)
  return @{ Process = $process; Directory = $directory; Executable = (Join-Path $directory 'mica_flutter.exe') }
}

try {
  $late = Start-InstallerFixture -Name 'late-dll' -Delay 1500 -Code 0
  Wait-MicaInstaller -Installer $late.Process -Executable $late.Executable
  if (-not (Test-Path -LiteralPath (Join-Path $late.Directory 'dependency.dll'))) {
    throw 'Wait returned before the installer finished its DLLs'
  }

  $bad = Start-InstallerFixture -Name 'failed' -Delay 10 -Code 7
  $rejected = $false
  try { Wait-MicaInstaller -Installer $bad.Process -Executable $bad.Executable }
  catch { $rejected = $_.Exception.Message -match 'code 7' }
  if (-not $rejected) { throw 'Failed installer was accepted because the EXE existed' }

  $missing = Start-InstallerFixture -Name 'missing' -Delay 10 -Code 0 -NoExecutable
  $rejected = $false
  try { Wait-MicaInstaller -Installer $missing.Process -Executable $missing.Executable }
  catch { $rejected = $_.Exception.Message -match 'exe not found' }
  if (-not $rejected) { throw 'Installer without an EXE was accepted' }

  $slow = Start-InstallerFixture -Name 'timeout' -Delay 5000 -Code 0
  $rejected = $false
  try { Wait-MicaInstaller -Installer $slow.Process -Executable $slow.Executable -TimeoutSeconds 1 }
  catch { $rejected = $_.Exception.Message -match 'did not finish' }
  if (-not $rejected) { throw 'Installer timeout was ignored' }
  Write-Host 'PASS: complete install, nonzero exit, missing EXE and timeout'
} finally {
  foreach ($process in $processes) {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue }
  }
  $resolved = [IO.Path]::GetFullPath($fixture)
  if ($resolved.StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase) -and
      (Split-Path -Leaf $resolved).StartsWith('mica-installer-wait-test-')) {
    Remove-Item -LiteralPath $resolved -Recurse -Force
  }
}
