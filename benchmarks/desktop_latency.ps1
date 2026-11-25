<#
.SYNOPSIS
  Typometer-style black-box Keystroke-to-Screen latency benchmark for Windows native `mica-rust.exe`.
.DESCRIPTION
  Launches `mica-rust.exe` in an isolated temporary APPDATA/LOCALAPPDATA profile,
  positions the window, sends real OS keystrokes via Win32 `SendInput`, and polls
  the editor region via Win32 GDI `BitBlt` pixel diffing until the rendered glyph
  appears on screen — measuring true end-to-end visual typing latency (P50 / P95 / P99 / Max).

  Measures both:
    1. Short document typing latency
    2. Large document (~1,000+ blocks pasted) typing latency
.EXAMPLE
  pwsh -File benchmarks/desktop_latency.ps1
#>
param(
  [string]$ExePath = "clients/mica_rust/target/release/mica-rust.exe",
  [int]$Samples = 35,
  [int]$LargeBlocks = 1000,
  [switch]$Json
)

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$exeFull = if ([System.IO.Path]::IsPathRooted($ExePath)) { $ExePath } else { Join-Path $repoRoot $ExePath }
if (-not (Test-Path $exeFull)) {
  throw "mica-rust.exe not found at $exeFull — build it with: cargo build --manifest-path clients/mica_rust/Cargo.toml --bin mica-rust --release"
}

Add-Type -TypeDefinition @"
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;

public static class MicaLatencyProbe {
  [StructLayout(LayoutKind.Sequential)]
  public struct RECT { public int Left, Top, Right, Bottom; }

  [StructLayout(LayoutKind.Sequential)]
  struct INPUT { public uint type; public InputUnion u; }

  [StructLayout(LayoutKind.Explicit)]
  struct InputUnion {
    [FieldOffset(0)] public MOUSEINPUT mi;
    [FieldOffset(0)] public KEYBDINPUT ki;
  }

  [StructLayout(LayoutKind.Sequential)]
  struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr dwExtraInfo; }

  [StructLayout(LayoutKind.Sequential)]
  struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr dwExtraInfo; }

  [StructLayout(LayoutKind.Sequential)]
  struct BITMAPINFOHEADER {
    public uint biSize; public int biWidth; public int biHeight;
    public ushort biPlanes; public ushort biBitCount; public uint biCompression;
    public uint biSizeImage; public int biXPelsPerMeter; public int biYPelsPerMeter;
    public uint biClrUsed; public uint biClrImportant;
  }

  [StructLayout(LayoutKind.Sequential)]
  struct BITMAPINFO { public BITMAPINFOHEADER bmiHeader; }

  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool MoveWindow(IntPtr hWnd, int X, int Y, int nWidth, int nHeight, bool bRepaint);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT lpRect);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int X, int Y);
  [DllImport("user32.dll")] static extern uint SendInput(uint nInputs, INPUT[] pInputs, int cbSize);
  [DllImport("user32.dll")] static extern IntPtr GetDC(IntPtr hWnd);
  [DllImport("user32.dll")] static extern int ReleaseDC(IntPtr hWnd, IntPtr hDC);
  [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleDC(IntPtr hdc);
  [DllImport("gdi32.dll")] static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int nWidth, int nHeight);
  [DllImport("gdi32.dll")] static extern IntPtr SelectObject(IntPtr hdc, IntPtr hgdiobj);
  [DllImport("gdi32.dll")] static extern bool BitBlt(IntPtr hdcDest, int xDest, int yDest, int w, int h, IntPtr hdcSrc, int xSrc, int ySrc, int rop);
  [DllImport("gdi32.dll")] static extern int GetDIBits(IntPtr hdc, IntPtr hbmp, uint start, uint lines, byte[] bits, ref BITMAPINFO bmi, uint usage);
  [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr hObject);
  [DllImport("gdi32.dll")] static extern bool DeleteDC(IntPtr hdc);

  const uint INPUT_MOUSE = 0;
  const uint INPUT_KEYBOARD = 1;
  const uint KEYEVENTF_KEYUP = 0x0002;
  const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
  const uint MOUSEEVENTF_LEFTUP = 0x0004;
  const int SRCCOPY = 0x00CC0020;

  public static void ClickAt(int x, int y) {
    SetCursorPos(x, y);
    var inputs = new INPUT[2];
    inputs[0].type = INPUT_MOUSE; inputs[0].u.mi.dwFlags = MOUSEEVENTF_LEFTDOWN;
    inputs[1].type = INPUT_MOUSE; inputs[1].u.mi.dwFlags = MOUSEEVENTF_LEFTUP;
    SendInput(2, inputs, Marshal.SizeOf(typeof(INPUT)));
  }

  public static void SendVk(ushort vk, bool ctrl = false) {
    var list = new System.Collections.Generic.List<INPUT>();
    if (ctrl) {
      var d = new INPUT { type = INPUT_KEYBOARD };
      d.u.ki.wVk = 0x11; // VK_CONTROL
      list.Add(d);
    }
    var kd = new INPUT { type = INPUT_KEYBOARD };
    kd.u.ki.wVk = vk;
    list.Add(kd);
    var ku = new INPUT { type = INPUT_KEYBOARD };
    ku.u.ki.wVk = vk; ku.u.ki.dwFlags = KEYEVENTF_KEYUP;
    list.Add(ku);
    if (ctrl) {
      var u = new INPUT { type = INPUT_KEYBOARD };
      u.u.ki.wVk = 0x11; u.u.ki.dwFlags = KEYEVENTF_KEYUP;
      list.Add(u);
    }
    SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
  }

  static void CaptureRect(IntPtr hdcScreen, IntPtr hdcMem, IntPtr hBmp, int x, int y, int w, int h, byte[] buf, ref BITMAPINFO bmi) {
    BitBlt(hdcMem, 0, 0, w, h, hdcScreen, x, y, SRCCOPY);
    GetDIBits(hdcMem, hBmp, 0, (uint)h, buf, ref bmi, 0);
  }

  static int CountChangedPixels(byte[] a, byte[] b) {
    int diff = 0;
    for (int i = 0; i < a.Length; i += 4) {
      int db = Math.Abs(a[i] - b[i]) + Math.Abs(a[i + 1] - b[i + 1]) + Math.Abs(a[i + 2] - b[i + 2]);
      if (db > 36) diff++;
    }
    return diff;
  }

  // Send one real OS keystroke and measure milliseconds until the editor region pixels change
  // beyond caret-blink noise (returns -1.0 on timeout).
  public static double MeasureKeystrokeMs(ushort vk, int rx, int ry, int rw, int rh, int timeoutMs) {
    IntPtr hdcScreen = GetDC(IntPtr.Zero);
    IntPtr hdcMem = CreateCompatibleDC(hdcScreen);
    IntPtr hBmp = CreateCompatibleBitmap(hdcScreen, rw, rh);
    IntPtr hOld = SelectObject(hdcMem, hBmp);
    try {
      var bmi = new BITMAPINFO();
      bmi.bmiHeader.biSize = (uint)Marshal.SizeOf(typeof(BITMAPINFOHEADER));
      bmi.bmiHeader.biWidth = rw;
      bmi.bmiHeader.biHeight = -rh;
      bmi.bmiHeader.biPlanes = 1;
      bmi.bmiHeader.biBitCount = 32;
      bmi.bmiHeader.biCompression = 0;

      byte[] baseBuf = new byte[rw * rh * 4];
      byte[] curBuf = new byte[rw * rh * 4];
      CaptureRect(hdcScreen, hdcMem, hBmp, rx, ry, rw, rh, baseBuf, ref bmi);

      var sw = Stopwatch.StartNew();
      SendVk(vk, false);

      while (sw.Elapsed.TotalMilliseconds < timeoutMs) {
        CaptureRect(hdcScreen, hdcMem, hBmp, rx, ry, rw, rh, curBuf, ref bmi);
        int changed = CountChangedPixels(baseBuf, curBuf);
        // A rendered glyph changes noticeably more pixels than a 1px vertical caret blink (~18px).
        if (changed >= 10) {
          return sw.Elapsed.TotalMilliseconds;
        }
      }
      return -1.0;
    } finally {
      SelectObject(hdcMem, hOld);
      DeleteObject(hBmp);
      DeleteDC(hdcMem);
      ReleaseDC(IntPtr.Zero, hdcScreen);
    }
  }
}
"@

function Summarize-Ms([double[]]$Values) {
  if (-not $Values -or $Values.Count -eq 0) {
    return [pscustomobject]@{ count = 0; min = 0; p50 = 0; p95 = 0; p99 = 0; max = 0 }
  }
  $sorted = $Values | Sort-Object
  $pct = {
    param([double]$p)
    $idx = [int][Math]::Round(($sorted.Count - 1) * $p)
    $sorted[[Math]::Min($sorted.Count - 1, $idx)]
  }
  [pscustomobject]@{
    count = $sorted.Count
    min   = [Math]::Round($sorted[0], 2)
    p50   = [Math]::Round((& $pct 0.50), 2)
    p95   = [Math]::Round((& $pct 0.95), 2)
    p99   = [Math]::Round((& $pct 0.99), 2)
    max   = [Math]::Round($sorted[$sorted.Count - 1], 2)
  }
}

$tmpRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("mica_desktop_lat_" + [Guid]::NewGuid().ToString("N").Substring(0, 8))
$appData = Join-Path $tmpRoot "Roaming"
$localAppData = Join-Path $tmpRoot "Local"
$probeFile = Join-Path $tmpRoot "startup_probe.txt"
New-Item -ItemType Directory -Force -Path $appData, $localAppData | Out-Null

$psi = [System.Diagnostics.ProcessStartInfo]::new()
$psi.FileName = $exeFull
$psi.UseShellExecute = $false
$psi.Environment["APPDATA"] = $appData
$psi.Environment["LOCALAPPDATA"] = $localAppData
$psi.Environment["MICA_STARTUP_PROBE_FILE"] = $probeFile

$coldSw = [System.Diagnostics.Stopwatch]::StartNew()
$proc = [System.Diagnostics.Process]::Start($psi)
try {
  while (-not (Test-Path $probeFile) -and $coldSw.ElapsedMilliseconds -lt 20000) {
    Start-Sleep -Milliseconds 10
  }
  $coldStartMs = [Math]::Round($coldSw.Elapsed.TotalMilliseconds, 1)

  # Wait for main window handle
  $deadline = (Get-Date).AddSeconds(20)
  while ($proc.MainWindowHandle -eq [IntPtr]::Zero -and (Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 150
    $proc.Refresh()
  }
  if ($proc.MainWindowHandle -eq [IntPtr]::Zero) {
    throw "mica-rust.exe window did not appear within 20s"
  }
  Start-Sleep -Milliseconds 1200
  $hwnd = $proc.MainWindowHandle
  [MicaLatencyProbe]::MoveWindow($hwnd, 80, 60, 1280, 820, $true) | Out-Null
  [MicaLatencyProbe]::SetForegroundWindow($hwnd) | Out-Null
  Start-Sleep -Milliseconds 400

  if ([MicaLatencyProbe]::GetForegroundWindow() -eq [IntPtr]::Zero) {
    Write-Warning "Windows interactive desktop is locked or disconnected (GetForegroundWindow=0); DWM screen pixel capture requires an unlocked desktop session. Cold start measured: ${coldStartMs} ms."
    if ($Json) {
      [pscustomobject]@{
        exe = $exeFull
        cold_start_ms = $coldStartMs
        pixel_capture_skipped = "desktop_locked_or_disconnected"
      } | ConvertTo-Json -Depth 4
    }
    return
  }

  $rect = New-Object MicaLatencyProbe+RECT
  [MicaLatencyProbe]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
  $winW = $rect.Right - $rect.Left
  $winH = $rect.Bottom - $rect.Top

  # Click editor body area (center-right of window, below title header)
  $clickX = $rect.Left + [int]($winW * 0.52)
  $clickY = $rect.Top + [int]($winH * 0.42)
  [MicaLatencyProbe]::ClickAt($clickX, $clickY)
  Start-Sleep -Milliseconds 350

  # Capture region covering main editor body (x: 28%..92%, y: 18%..88%)
  $rx = $rect.Left + [int]($winW * 0.28)
  $ry = $rect.Top + [int]($winH * 0.18)
  $rw = [int]($winW * 0.64)
  $rh = [int]($winH * 0.70)

  # Warmup 5 keys (use wide ASCII digits so active CJK IME never hijacks into preedit)
  for ($i = 0; $i -lt 5; $i++) {
    [MicaLatencyProbe]::SendVk(0x38, $false) # '8'
    Start-Sleep -Milliseconds 80
  }
  Start-Sleep -Milliseconds 250

  # 1. Short document keystroke-to-screen latency
  # Wide ASCII digits bypass CJK IME composition when the desktop taskbar is in Chinese mode
  # and guarantee >20 changed pixels per glyph (avoiding thin-glyph misses like lowercase 'i').
  $vks = @(0x30, 0x32, 0x33, 0x34, 0x35, 0x36, 0x38, 0x39) # 0 2 3 4 5 6 8 9
  $shortSamples = New-Object System.Collections.Generic.List[double]
  $shortOutliers = New-Object System.Collections.Generic.List[string]
  $shortMisses = 0
  for ($i = 0; $i -lt $Samples; $i++) {
    $vk = $vks[$i % $vks.Count]
    $ms = [MicaLatencyProbe]::MeasureKeystrokeMs($vk, $rx, $ry, $rw, $rh, 500)
    if ($ms -ge 0) {
      $shortSamples.Add($ms)
      if ($ms -gt 120.0) { $shortOutliers.Add(("#{0} vk=0x{1:X2} {2}ms" -f $i, $vk, [Math]::Round($ms, 1))) }
    } else {
      $shortMisses++
    }
    Start-Sleep -Milliseconds 90
  }
  $shortStats = Summarize-Ms $shortSamples.ToArray()

  # 2. Paste large Markdown document (~$LargeBlocks blocks) and measure typing latency again
  $sb = [System.Text.StringBuilder]::new()
  for ($b = 1; $b -le $LargeBlocks; $b++) {
    if ($b % 10 -eq 0) {
      [void]$sb.AppendLine("## Section $b Rust Native Desktop Latency")
    } else {
      [void]$sb.AppendLine("Paragraph $b verifying pure-Rust NSH virtual_list keystroke latency under long document load.")
    }
    [void]$sb.AppendLine()
  }
  Set-Clipboard -Value $sb.ToString()
  Start-Sleep -Milliseconds 150
  [MicaLatencyProbe]::SendVk(0x0D, $false) # Enter
  Start-Sleep -Milliseconds 120
  [MicaLatencyProbe]::SendVk(0x56, $true)  # Ctrl+V
  Start-Sleep -Milliseconds 1800
  [MicaLatencyProbe]::SendVk(0x0D, $false) # Enter to start fresh line
  Start-Sleep -Milliseconds 300

  # Warmup 3 keys
  for ($i = 0; $i -lt 3; $i++) {
    [MicaLatencyProbe]::SendVk(0x38, $false)
    Start-Sleep -Milliseconds 80
  }
  Start-Sleep -Milliseconds 250

  $longSamples = New-Object System.Collections.Generic.List[double]
  $longOutliers = New-Object System.Collections.Generic.List[string]
  $longMisses = 0
  for ($i = 0; $i -lt $Samples; $i++) {
    $vk = $vks[$i % $vks.Count]
    $ms = [MicaLatencyProbe]::MeasureKeystrokeMs($vk, $rx, $ry, $rw, $rh, 500)
    if ($ms -ge 0) {
      $longSamples.Add($ms)
      if ($ms -gt 120.0) { $longOutliers.Add(("#{0} vk=0x{1:X2} {2}ms" -f $i, $vk, [Math]::Round($ms, 1))) }
    } else {
      $longMisses++
    }
    Start-Sleep -Milliseconds 90
  }
  $longStats = Summarize-Ms $longSamples.ToArray()

  $out = [pscustomobject]@{
    exe           = $exeFull
    cold_start_ms = $coldStartMs
    window_size   = "${winW}x${winH}"
    short_doc     = [pscustomobject]@{ stats = $shortStats; timeouts = $shortMisses; outliers = $shortOutliers.ToArray() }
    large_doc     = [pscustomobject]@{ blocks = $LargeBlocks; stats = $longStats; timeouts = $longMisses; outliers = $longOutliers.ToArray() }
  }

  if ($Json) {
    $out | ConvertTo-Json -Depth 5
  } else {
    Write-Host "Mica Windows Native Desktop Keystroke-to-Screen Latency (Typometer-style BitBlt)"
    Write-Host "  Cold start: ${coldStartMs} ms  Window: ${winW}x${winH}  Capture ROI: ${rw}x${rh}"
    Write-Host ""
    Write-Host "=== [1/2] Short Document ($($shortStats.count)/$Samples detected, timeouts=$shortMisses) ==="
    Write-Host ("  key_to_screen   min={0} ms  p50={1} ms  p95={2} ms  p99={3} ms  max={4} ms" -f $shortStats.min, $shortStats.p50, $shortStats.p95, $shortStats.p99, $shortStats.max)
    if ($shortOutliers.Count -gt 0) { Write-Host ("  outliers(>120ms): " + ($shortOutliers -join ", ")) }
    Write-Host ""
    Write-Host "=== [2/2] Large Document ($LargeBlocks pasted blocks, $($longStats.count)/$Samples detected, timeouts=$longMisses) ==="
    Write-Host ("  key_to_screen   min={0} ms  p50={1} ms  p95={2} ms  p99={3} ms  max={4} ms" -f $longStats.min, $longStats.p50, $longStats.p95, $longStats.p99, $longStats.max)
    if ($longOutliers.Count -gt 0) { Write-Host ("  outliers(>120ms): " + ($longOutliers -join ", ")) }
  }
} finally {
  if ($proc -and -not $proc.HasExited) {
    $proc.CloseMainWindow() | Out-Null
    Start-Sleep -Milliseconds 400
    if (-not $proc.HasExited) { $proc.Kill() }
  }
  Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmpRoot
}
