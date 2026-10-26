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
///
/// The HWND is found by class AND owning process (issue #142): the runner class
/// name is the default for every Flutter Windows app, so a plain `FindWindowW`
/// can return ANOTHER app's window and we would save its snap state as ours.
library;

import 'dart:ffi';
import 'dart:io' show Platform, pid;

import 'package:ffi/ffi.dart';
import 'package:flutter/foundation.dart' show visibleForTesting;

typedef _IsWindowArrangedC = Int32 Function(IntPtr hWnd);
typedef _IsWindowArrangedDart = int Function(int hWnd);

typedef _FindWindowExC =
    IntPtr Function(
      IntPtr parent,
      IntPtr childAfter,
      Pointer<Utf16> cls,
      Pointer<Utf16> title,
    );
typedef _FindWindowExDart =
    int Function(
      int parent,
      int childAfter,
      Pointer<Utf16> cls,
      Pointer<Utf16> title,
    );

typedef _GetWindowThreadProcessIdC =
    Uint32 Function(IntPtr hWnd, Pointer<Uint32> processId);
typedef _GetWindowThreadProcessIdDart =
    int Function(int hWnd, Pointer<Uint32> processId);

/// The class the Flutter runner registers (`windows/runner/win32_window.cpp`).
/// Looking the window up by class beats `GetForegroundWindow`, which is only
/// ours while we happen to be focused — and a move can finish after focus left.
const String _kRunnerWindowClass = 'FLUTTER_RUNNER_WIN32_WINDOW';

/// Walk the windows of [_kRunnerWindowClass] and return the first one owned by
/// [currentPid], or 0 when there is none.
///
/// Split out from the FFI call because it is the part that can be wrong in a
/// way a headless test can see: `FindWindowExW`'s first result is simply the
/// top-most window of that class, which may belong to any other Flutter app
/// running on the machine. `ownerPid` is `GetWindowThreadProcessId`'s
/// out-parameter.
int _findOwnedWindow({
  required int currentPid,
  required int Function(int after) nextOfClass,
  required int Function(int hwnd) ownerPid,
}) {
  var after = 0;
  for (var i = 0; i < 4096; i++) {
    final hwnd = nextOfClass(after);
    if (hwnd == 0) return 0;
    if (ownerPid(hwnd) == currentPid) return hwnd;
    after = hwnd;
  }
  return 0;
}

bool isWindowSnapped({
  @visibleForTesting int Function(int after)? nextOfClass,
  @visibleForTesting int Function(int hwnd)? ownerPid,
  @visibleForTesting bool Function(int hwnd)? arranged,
}) {
  if (!Platform.isWindows) return false;
  if (nextOfClass != null && ownerPid != null && arranged != null) {
    final hwnd = _findOwnedWindow(
      currentPid: pid,
      nextOfClass: nextOfClass,
      ownerPid: ownerPid,
    );
    return hwnd != 0 && arranged(hwnd);
  }
  Pointer<Utf16>? cls;
  Pointer<Uint32>? ownerPidOut;
  try {
    final user32 = DynamicLibrary.open('user32.dll');
    if (!user32.providesSymbol('IsWindowArranged')) return false;
    final isArranged = user32
        .lookupFunction<_IsWindowArrangedC, _IsWindowArrangedDart>(
          'IsWindowArranged',
        );
    final findWindowEx = user32
        .lookupFunction<_FindWindowExC, _FindWindowExDart>('FindWindowExW');
    final getWindowThreadProcessId = user32
        .lookupFunction<
          _GetWindowThreadProcessIdC,
          _GetWindowThreadProcessIdDart
        >('GetWindowThreadProcessId');

    final clsPtr = _kRunnerWindowClass.toNativeUtf16();
    cls = clsPtr;
    final pidPtr = calloc<Uint32>();
    ownerPidOut = pidPtr;
    final hwnd = _findOwnedWindow(
      currentPid: pid,
      nextOfClass: (after) => findWindowEx(0, after, clsPtr, nullptr),
      ownerPid: (h) {
        pidPtr.value = 0;
        if (getWindowThreadProcessId(h, pidPtr) == 0) return 0;
        return pidPtr.value;
      },
    );
    if (hwnd == 0) return false;
    return isArranged(hwnd) != 0;
  } catch (_) {
    return false;
  } finally {
    if (cls != null) calloc.free(cls);
    if (ownerPidOut != null) calloc.free(ownerPidOut);
  }
}
