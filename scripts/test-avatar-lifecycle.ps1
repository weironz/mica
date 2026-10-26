param([string]$Filter = '', [switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
$Run = Join-Path ([IO.Path]::GetTempPath()) ('mica-avatar-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Run | Out-Null
# Fresh name per run: a re-entrant or concurrent run can never drop a database
# it did not create, and cleanup drops only this run's own name.
$DbName = 'mica_fix_e2_avatar_' + ([guid]::NewGuid().ToString('N').Substring(0, 12))
# Ports are drawn per run too: two overlapping runs must not share a listener.
$ApiPort = 18300 + (Get-Random -Maximum 200)
$S3Port = $ApiPort + 1000
# The runner accepts a private CARGO_TARGET_DIR so parallel cargo users do not
# contend on the repo's target/ lock; unset, it builds where `cargo` would.
$BinRoot = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Repo 'target' }
$api = $null
$s3 = $null
$result = 1
try {
  if (-not $SkipBuild) { & cargo build -p mica-api-server; if ($LASTEXITCODE) { throw 'build failed' } }
  # The database is minted fresh by name above; nothing here drops a
  # pre-existing database.
  & docker exec mica-test-pg createdb -U mica $DbName
  if ($LASTEXITCODE) { throw 'fresh database required' }
  $env:APP_ENV='test'
  $env:HTTP_ADDR="127.0.0.1:$ApiPort"
  $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:55432/$DbName"
  $env:DATABASE_MAX_CONNECTIONS='10'
  $env:JWT_SECRET='avatar-lifecycle-secret-not-a-real-key'
  $env:MICA_SEED_TEST_USER='avatar-seed@mica.test:avatar-password-123'
  $env:S3_ENDPOINT="http://127.0.0.1:$S3Port"
  $env:S3_REGION='us-east-1'
  $env:S3_BUCKET='mica'
  $env:S3_ACCESS_KEY_ID='probe'
  $env:S3_SECRET_ACCESS_KEY='probe-secret'
  $env:S3_FORCE_PATH_STYLE='true'
  $env:APPDATA=$Run
  $env:LOCALAPPDATA=$Run
  $env:TEMP=$Run
  $env:TMP=$Run
  $env:HTTP_PROXY=''
  $env:http_proxy=''
  $env:AVATAR_S3_PORT="$S3Port"
  $env:AVATAR_TEST_API="http://127.0.0.1:$ApiPort"
  $bun=(Get-Command bun).Source
  $s3=Start-Process $bun -ArgumentList 'run',(Join-Path $Repo 'scripts/test-avatar-s3.mjs') -WorkingDirectory $Run -RedirectStandardOutput (Join-Path $Run 's3.log') -RedirectStandardError (Join-Path $Run 's3.err') -PassThru
  $api=Start-Process (Join-Path $BinRoot 'debug\mica-api-server.exe') -WorkingDirectory $Run -RedirectStandardOutput (Join-Path $Run 'api.log') -RedirectStandardError (Join-Path $Run 'api.err') -PassThru
  $ready=$false
  for($i=0;$i -lt 150;$i++) {
    try { $null=Invoke-RestMethod "$env:AVATAR_TEST_API/api/health"; $ready=$true; break } catch { Start-Sleep -Milliseconds 200 }
  }
  if(-not $ready) { throw "API not ready; logs: $Run" }
  if($Filter) { & bun test (Join-Path $Repo 'scripts/test-avatar-lifecycle.mjs') -t $Filter } else { & bun test (Join-Path $Repo 'scripts/test-avatar-lifecycle.mjs') }
  $result=$LASTEXITCODE
} finally {
  if($api) { Stop-Process -Id $api.Id -Force -ErrorAction SilentlyContinue }
  if($s3) { Stop-Process -Id $s3.Id -Force -ErrorAction SilentlyContinue }
  & docker exec mica-test-pg dropdb -U mica --force $DbName 2>&1 | Out-Null
}
exit $result
