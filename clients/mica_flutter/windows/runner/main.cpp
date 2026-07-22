#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>

#include <string>

#include "flutter_window.h"
#include "utils.h"

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  // Single-instance guard. Two instances share one store.db and clobber each
  // other's local documents (whole-document last-writer-wins), and the "close =
  // minimize to tray" mode means a second shortcut click on an already-hidden
  // app would silently spawn a duplicate. A named mutex detects a live instance;
  // when one exists we surface its window and bail before creating our own.
  //
  // Session-scoped (Local namespace) on purpose: FindWindowW only sees windows
  // in the caller's session, so a Global guard matching an instance in another
  // session would just exit with nothing to bring forward. Same-user
  // cross-session double-open (one APPDATA / store.db shared over RDP) is not
  // covered here -- a file lock on store.db would be the tool for that.
  //
  // Fail open: if CreateMutexW itself fails (returns null) we proceed normally
  // rather than block the only instance from launching.
  HANDLE single_instance_mutex =
      ::CreateMutexW(nullptr, TRUE, L"Local\\MicaSingleInstance");
  if (single_instance_mutex != nullptr &&
      ::GetLastError() == ERROR_ALREADY_EXISTS) {
    // Class name must stay in sync with kWindowClassName in win32_window.cpp.
    // Title is what window_manager applies (WindowOptions.title == "Mica"),
    // with a fallback to the native creation title used before Dart runs, then
    // a final class-only match for robustness against a startup race.
    HWND existing = ::FindWindowW(L"FLUTTER_RUNNER_WIN32_WINDOW", L"Mica");
    if (existing == nullptr) {
      existing = ::FindWindowW(L"FLUTTER_RUNNER_WIN32_WINDOW", L"mica_flutter");
    }
    if (existing == nullptr) {
      existing = ::FindWindowW(L"FLUTTER_RUNNER_WIN32_WINDOW", nullptr);
    }
    if (existing != nullptr) {
      // The window may be hidden in the tray (ShowWindow SW_HIDE) or minimized:
      // unhide it, restore if iconic, then pull it to the foreground.
      ::ShowWindow(existing, SW_SHOW);
      if (::IsIconic(existing)) {
        ::ShowWindow(existing, SW_RESTORE);
      }
      ::SetForegroundWindow(existing);
    }
    ::CloseHandle(single_instance_mutex);
    return EXIT_SUCCESS;
  }

  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"data");

  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1280, 720);
  if (!window.Create(L"mica_flutter", origin, size)) {
    return EXIT_FAILURE;
  }
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  return EXIT_SUCCESS;
}
