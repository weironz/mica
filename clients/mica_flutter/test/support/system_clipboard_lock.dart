// Cross-process lock for test suites that touch the REAL system clipboard.
//
// windows_clipboard_copy_test verifies native formats by writing to the OS
// clipboard and reading it back; editor_cut_transaction_test deliberately
// HOLDS the clipboard open (test/support/hold_clipboard.ps1) to exercise the
// busy-clipboard fallback. Every test file runs in its own flutter_tester
// process, and `flutter test` runs files in parallel by default, so an
// in-process lock cannot protect the single OS-wide clipboard. Without one,
// the hold outlasts the other suite's OpenClipboard retries (the readback
// suite failed this way on 2026-11-15), and aborting a stuck run can leave
// the clipboard locked for every process on the machine.
//
// A named Win32 mutex serializes these suites across processes. If a test
// process is killed while holding it, the kernel abandons the mutex and the
// next waiter still acquires it (WAIT_ABANDONED), so a crash cannot wedge
// later runs. Waiting polls instead of blocking the isolate, so the test
// runner's channel stays responsive.
import 'dart:async';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';

final DynamicLibrary _kernel32 = DynamicLibrary.open('kernel32.dll');

final Pointer Function(Pointer<Void>, int, Pointer<Utf16>) _createMutex =
    _kernel32.lookupFunction<
      Pointer Function(Pointer<Void>, Int32, Pointer<Utf16>),
      Pointer Function(Pointer<Void>, int, Pointer<Utf16>)
    >('CreateMutexW');
final int Function(Pointer, int) _waitForSingleObject = _kernel32
    .lookupFunction<
      Uint32 Function(Pointer, Uint32),
      int Function(Pointer, int)
    >('WaitForSingleObject');
final int Function(Pointer) _releaseMutex = _kernel32
    .lookupFunction<Int32 Function(Pointer), int Function(Pointer)>(
      'ReleaseMutex',
    );
final int Function(Pointer) _closeHandle = _kernel32
    .lookupFunction<Int32 Function(Pointer), int Function(Pointer)>(
      'CloseHandle',
    );
final int Function() _getLastError = _kernel32
    .lookupFunction<Uint32 Function(), int Function()>('GetLastError');

const int _waitAbandoned = 0x80;
const int _waitFailed = 0xFFFFFFFF;
const Duration _pollTimeout = Duration(milliseconds: 200);
const Duration _maxWait = Duration(minutes: 10); // hold_clipboard.ps1: 3 min.

Pointer? _handle;

/// Waits until this process owns the cross-process clipboard lock.
Future<void> acquireSystemClipboardLock() async {
  if (!Platform.isWindows || _handle != null) return;
  final handle = using((arena) {
    final name = 'mica-system-clipboard-test-lock'.toNativeUtf16(
      allocator: arena,
    );
    final created = _createMutex(nullptr, 0, name);
    if (created == nullptr) {
      throw StateError('CreateMutexW failed: ${_getLastError()}');
    }
    return created;
  });
  final deadline = DateTime.now().add(_maxWait);
  while (true) {
    final result = _waitForSingleObject(handle, _pollTimeout.inMilliseconds);
    // WAIT_OBJECT_0 and WAIT_ABANDONED both grant ownership.
    if (result == 0 || result == _waitAbandoned) {
      _handle = handle;
      return;
    }
    if (result == _waitFailed || DateTime.now().isAfter(deadline)) {
      _closeHandle(handle);
      throw StateError('system clipboard lock wait failed: $result');
    }
    await Future<void>.delayed(const Duration(milliseconds: 50));
  }
}

/// Releases the lock taken by [acquireSystemClipboardLock].
void releaseSystemClipboardLock() {
  final handle = _handle;
  if (handle == null) return;
  _handle = null;
  _releaseMutex(handle);
  _closeHandle(handle);
}
