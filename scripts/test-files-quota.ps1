param([string]$Filter = '', [switch]$SkipBuild, [switch]$OwnerQuotaDisabled, [switch]$ModuleTests, [string]$DbName = 'mica_fix_c_122', [int]$ApiPort = 18222, [int]$S3Port = 19222)
$ErrorActionPreference = 'Stop'
$Repo = Split-Path -Parent $PSScriptRoot
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR=Join-Path $Repo '.tmp-probes/fixc-target' }
$Run = Join-Path ([IO.Path]::GetTempPath()) ('mica-files-quota-' + [guid]::NewGuid())
New-Item -ItemType Directory -Path $Run | Out-Null
$api = $null
$s3 = $null
$createdDatabase = $false
try {
  if (-not $SkipBuild) { & cargo build -j4 -p mica-api-server --bin mica-api-server --example file_gc_test_driver; if ($LASTEXITCODE) { throw 'build failed' } }
  & docker exec mica-test-pg createdb -U mica $DbName
  if ($LASTEXITCODE) { throw 'fresh database required' }
  $createdDatabase = $true
  $env:APP_ENV='test'
  $env:HTTP_ADDR="127.0.0.1:$ApiPort"
  $env:DATABASE_URL="postgres://mica:mica@127.0.0.1:55432/$DbName"
  $env:JWT_SECRET='quota-regression-secret-not-a-real-key'
  $env:MICA_SEED_TEST_USER='quota-seed@mica.test:quota-password-123'
  $env:MICA_WORKSPACE_QUOTA_BYTES='1000'
  $env:MICA_OWNER_QUOTA_BYTES='1000'
  if($OwnerQuotaDisabled) { $env:MICA_OWNER_QUOTA_BYTES='0' }
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
  $env:FAKE_S3_PORT="$S3Port"
  $env:FILES_TEST_API="http://127.0.0.1:$ApiPort"
  $env:FILES_TEST_GC=Join-Path $env:CARGO_TARGET_DIR 'debug/examples/file_gc_test_driver.exe'
  if(-not (Test-Path $env:FILES_TEST_GC)) { throw "missing freshly built GC driver; run without -SkipBuild" }
  $bun=(Get-Command bun).Source
  $env:HTTP_PROXY="http://127.0.0.1:$S3Port"
  $env:http_proxy=$env:HTTP_PROXY
  $env:NO_PROXY='127.0.0.1,localhost'
  $env:no_proxy=$env:NO_PROXY
  $s3=Start-Process $bun -ArgumentList 'run',(Join-Path $Repo 'scripts/test-files-s3.mjs') -WorkingDirectory $Run -RedirectStandardOutput (Join-Path $Run 's3.log') -RedirectStandardError (Join-Path $Run 's3.err') -PassThru
  $api=Start-Process (Join-Path $env:CARGO_TARGET_DIR 'debug/mica-api-server.exe') -WorkingDirectory $Run -RedirectStandardOutput (Join-Path $Run 'api.log') -RedirectStandardError (Join-Path $Run 'api.err') -PassThru
  $ready=$false
  for($i=0;$i -lt 100;$i++) {
    try { $null=Invoke-RestMethod "$env:FILES_TEST_API/api/health"; $ready=$true; break } catch { Start-Sleep -Milliseconds 200 }
  }
  if(-not $ready) { throw "API not ready; logs: $Run" }
  if($Filter) { & bun test (Join-Path $Repo 'scripts/test-files-quota.mjs') -t $Filter } else { & bun test (Join-Path $Repo 'scripts/test-files-quota.mjs') }
  $result=$LASTEXITCODE
  if($ModuleTests -and $result -eq 0) {
    & cargo test -j4 -p mica-api-server --bin mica-api-server routes::files::
    if($LASTEXITCODE) { $result=$LASTEXITCODE }
    if($result -eq 0) { & cargo test -j4 -p mica-api-server --bin mica-api-server blob_gc::; $result=$LASTEXITCODE }
  }
} finally {
  if($api) { Stop-Process -Id $api.Id -Force -ErrorAction SilentlyContinue }
  if($s3) { Stop-Process -Id $s3.Id -Force -ErrorAction SilentlyContinue }
  if($createdDatabase) { & docker exec mica-test-pg dropdb -U mica --force $DbName }
}
exit $result