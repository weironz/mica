# Quick-launch Mica for manual verification (from the current working tree).
#
#   .\run.ps1            # desktop (Windows) — local mode, no backend needed
#   .\run.ps1 web        # web in Chrome (cloud login needed for data)
#
# Hot reload: after I change code, press  r  in this terminal to see it live
# (no rebuild). Press  R  for a full restart,  q  to quit.
#
# Desktop uses the offline "local" world out of the box, so you can create
# folders/pages and try things immediately — no server or login required.
param([string]$target = "windows")
$ErrorActionPreference = "Stop"

$device = switch ($target) {
  "web"     { "chrome" }
  "chrome"  { "chrome" }
  "windows" { "windows" }
  "desktop" { "windows" }
  default   { $target }
}

Write-Host "Launching Mica on '$device'  (hot reload: r  |  restart: R  |  quit: q)" -ForegroundColor Cyan
Push-Location "$PSScriptRoot\clients\mica_flutter"
try {
  flutter run -d $device
} finally {
  Pop-Location
}
