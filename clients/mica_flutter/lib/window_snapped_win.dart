/// Is the window currently held in place by Aero Snap?
///
/// Snap (Win+Left/Right/Up, or dragging to a screen edge) is a FOURTH window
/// state, and the one the bounds persister used to miss: a snapped window
/// reports `showCmd == SW_SHOWNORMAL`, not maximized, not minimized, not
/// fullscreen. So the snapped half-screen rect got recorded as the window's
/// restore geometry, and every launch after that reopened a half-width window
/// pinned to one edge, with nothing to explain it.
///
/// `IsWindowArranged` is the documented test for exactly this (Windows 10 1903+
/// — Electron exposes the same call as `win.isSnapped()`). Verified live:
/// True for half- and quarter-snap, False when floating or maximized.
///
/// Resolved dynamically rather than linked: the Win32 docs contradict
/// themselves on whether it has an import library ("this function does not have
/// an associated header file or library file" in Remarks, versus a requirements
/// table naming winuser.h/User32.lib) and the page is still flagged prerelease.
/// Electron resolves it dynamically for the same reason. Anything unexpected —
/// older Windows, lookup failure, non-Windows — degrades to false, i.e. exactly
/// the previous behaviour.
library;

import 'dart:ffi';
import 'dart:io' show Platform;

import 'package:ffi/ffi.dart';

typedef _IsWindowArrangedC = Int32 Function(IntPtr hWnd);
typedef _IsWindowArrangedDart = int Function(int hWnd);

typedef _FindWindowC = IntPtr Function(Pointer<Utf16> cls, Pointer<Utf16> title);
typedef _FindWindowDart = int Function(Pointer<Utf16> cls, Pointer<Utf16> title);

/// The class the Flutter runner registers (`windows/runner/win32_window.cpp`).
/// Looking the window up by class beats `GetForegroundWindow`, which is only
/// ours while we happen to be focused — and a move can finish after focus left.
const String _kRunnerWindowClass = 'FLUTTER_RUNNER_WIN32_WINDOW';

bool isWindowSnapped() {
  if (!Platform.isWindows) return false;
  Pointer<Utf16>? cls;
  try {
    final user32 = DynamicLibrary.open('user32.dll');
    if (!user32.providesSymbol('IsWindowArranged')) return false;
    final isArranged =
        user32.lookupFunction<_IsWindowArrangedC, _IsWindowArrangedDart>(
            'IsWindowArranged');
    final findWindow =
        user32.lookupFunction<_FindWindowC, _FindWindowDart>('FindWindowW');

    cls = _kRunnerWindowClass.toNativeUtf16();
    final hwnd = findWindow(cls, nullptr);
    if (hwnd == 0) return false;
    return isArranged(hwnd) != 0;
  } catch (_) {
    return false;
  } finally {
    if (cls != null) calloc.free(cls);
  }
}
