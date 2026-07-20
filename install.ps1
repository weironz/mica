# Mica CLI installer for Windows.
#
#   irm https://raw.githubusercontent.com/weironz/mica/main/install.ps1 | iex
#
# Re-run any time to update to the latest release. To pin a version:
#   $env:MICA_VERSION='0.5.4'; irm https://raw.githubusercontent.com/weironz/mica/main/install.ps1 | iex
#
# Installs mica-cli.exe to %LOCALAPPDATA%\Mica\bin and puts it on your PATH.
# Downloads the prebuilt binary from the GitHub release — nothing is built.

$ErrorActionPreference = 'Stop'
# Windows PowerShell 5.1 defaults to old TLS; the GitHub API/CDN need 1.2.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$repo = 'weironz/mica'
$installDir = Join-Path $env:LOCALAPPDATA 'Mica\bin'
$exe = Join-Path $installDir 'mica-cli.exe'
$ua = @{ 'User-Agent' = 'mica-cli-installer' }  # GitHub's API 403s a request with no UA.

# Resolve the download URL: an explicit MICA_VERSION derives it by convention,
# otherwise ask the API for the latest release and match the asset (robust
# against a naming change).
if ($env:MICA_VERSION) {
  $version = $env:MICA_VERSION -replace '^v', ''
  $url = "https://github.com/$repo/releases/download/v$version/mica-cli-$version-windows-x64.exe"
} else {
  $rel = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers $ua
  $version = $rel.tag_name -replace '^v', ''
  $asset = $rel.assets | Where-Object { $_.name -like 'mica-cli-*-windows-x64.exe' } | Select-Object -First 1
  if (-not $asset) { throw "no windows-x64 binary in release $($rel.tag_name)" }
  $url = $asset.browser_download_url
}

Write-Host "Installing mica-cli $version -> $exe"
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Invoke-WebRequest -Uri $url -OutFile $exe -UseBasicParsing

# Add the install dir to the USER PATH once (idempotent), and to THIS session so
# `mica-cli` works without reopening the terminal.
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($userPath -split ';') -notcontains $installDir) {
  $joined = if ([string]::IsNullOrEmpty($userPath)) { $installDir } else { "$userPath;$installDir" }
  [Environment]::SetEnvironmentVariable('Path', $joined, 'User')
  Write-Host "Added $installDir to your PATH (new terminals pick it up automatically)."
}
if (($env:Path -split ';') -notcontains $installDir) { $env:Path = "$env:Path;$installDir" }

& $exe --version
Write-Host ""
Write-Host "Installed. Next:"
Write-Host "  mica-cli auth login --server https://your-server.example.com --email you@example.com"
Write-Host "Re-run this line any time to update."
