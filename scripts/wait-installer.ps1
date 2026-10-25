# Wait for the complete installer, not the first file it extracts.
function Wait-MicaInstaller {
  param(
    [Parameter(Mandatory = $true)][System.Diagnostics.Process]$Installer,
    [Parameter(Mandatory = $true)][string]$Executable,
    [ValidateRange(1, 120)][int]$TimeoutSeconds = 120
  )
  if (-not $Installer.WaitForExit($TimeoutSeconds * 1000)) {
    throw "installer did not finish within ${TimeoutSeconds}s: $Executable"
  }
  $Installer.WaitForExit()
  $Installer.Refresh()
  if ($Installer.ExitCode -ne 0) {
    throw "installer exited with code $($Installer.ExitCode): $Executable"
  }
  if (-not (Test-Path -LiteralPath $Executable)) {
    throw "installed exe not found after installer completed: $Executable"
  }
}
