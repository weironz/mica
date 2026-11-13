param(
  [Parameter(Mandatory=$true)][string]$Before,
  [Parameter(Mandatory=$true)][string]$After,
  [Parameter(Mandatory=$true)][string]$Output,
  [int]$Pairs=20
)
$ErrorActionPreference='Stop'
# Read the existing native first-frame receipt; avoid stopwatch polling error.
# Each run is a fresh process with fresh app data. The OS file cache stays intact.
New-Item -Type Directory -Force $Output | Out-Null
$os=Get-CimInstance Win32_OperatingSystem
$cpu=Get-CimInstance Win32_Processor|Select-Object -First 1
@{recordedUtc=[DateTime]::UtcNow.ToString('o');os=$os.Caption;version=$os.Version;build=$os.BuildNumber;cpu=$cpu.Name;logicalProcessors=$cpu.NumberOfLogicalProcessors;totalMemoryKB=$os.TotalVisibleMemorySize;freeMemoryKB=$os.FreePhysicalMemory;pairs=$Pairs;cacheReset=$false;isolatedProfiles=$true}|ConvertTo-Json|Set-Content "$Output\environment.json" -Encoding UTF8
$paths=@{before=(Resolve-Path $Before).Path;after=(Resolve-Path $After).Path}
$records=@()
$names=@('APPDATA','LOCALAPPDATA','TEMP','TMP','MICA_STARTUP_PROBE_FILE','MICA_STARTUP_TIMING_FILE')
$old=@{}
foreach($name in $names){$old[$name]=[Environment]::GetEnvironmentVariable($name,'Process')}
try {
  # Keep launch order balanced to reduce drift from unrelated machine load.
  for($pair=0;$pair -lt $Pairs;$pair++) {
    $order=if($pair % 2 -eq 0){@('before','after')}else{@('after','before')}
    foreach($label in $order) {
      if(Get-Process mica_flutter -ErrorAction SilentlyContinue){throw 'A Mica instance is already running; leave it untouched and stop this measurement.'}
      $dir=Join-Path $Output ("{0:D2}-{1}" -f $pair,$label)
      foreach($child in @('roaming','local','temp')){New-Item -Type Directory -Force "$dir\$child"|Out-Null}
      $env:APPDATA="$dir\roaming";$env:LOCALAPPDATA="$dir\local"
      $env:TEMP="$dir\temp";$env:TMP=$env:TEMP
      $env:MICA_STARTUP_PROBE_FILE="$dir\first-frame.marker"
      $env:MICA_STARTUP_TIMING_FILE="$dir\timing.json"
      $app=$null
      try {
        $app=Start-Process -FilePath $paths[$label] -WorkingDirectory (Split-Path $paths[$label]) -PassThru
        $started=$app.StartTime.ToUniversalTime()
        $deadline=[Diagnostics.Stopwatch]::StartNew()
        while($true) {
          $app.Refresh()
          if($app.HasExited){throw "Measured application exited before receipt: $($app.ExitCode)"}
          if((Test-Path $env:MICA_STARTUP_PROBE_FILE) -and (Test-Path $env:MICA_STARTUP_TIMING_FILE)) {
            try {
              $marker=[IO.File]::ReadAllText($env:MICA_STARTUP_PROBE_FILE)
              $timing=[IO.File]::ReadAllText($env:MICA_STARTUP_TIMING_FILE)|ConvertFrom-Json
              if($marker.Trim() -eq 'first_frame' -and $timing.pid -eq $app.Id){
                if($timing.dpr -ne 1){throw 'Windows display is not at 100 percent'}
                break
              }
            } catch [IO.IOException] {}
          }
          if($deadline.Elapsed.TotalSeconds -gt 30){throw 'No first-frame receipt within 30 seconds'}
          Start-Sleep -Milliseconds 10
        }
        $frameTime=[IO.File]::GetLastWriteTimeUtc($env:MICA_STARTUP_PROBE_FILE)
        $records+=@{pair=$pair;variant=$label;pid=$app.Id;processCreatedUtc=$started.ToString('o');firstFrameWrittenUtc=$frameTime.ToString('o');processToFirstFrameMs=($frameTime-$started).TotalMilliseconds;timing=$timing}
        $records|ConvertTo-Json -Depth 8|Set-Content "$Output\runs.json" -Encoding UTF8
      } finally {
        if($app -and -not $app.HasExited){Stop-Process -Id $app.Id -Force;$app.WaitForExit()}
      }
    }
  }
} finally {
  foreach($name in $names){[Environment]::SetEnvironmentVariable($name,$old[$name],'Process')}
}
