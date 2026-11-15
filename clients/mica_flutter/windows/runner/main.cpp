#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <knownfolders.h>
#include <shlobj.h>
#include <windows.h>

#include <cstdint>
#include <string>
#include <vector>

#include "flutter_window.h"
#include "utils.h"

namespace {

// Absolute, lower-case form of `path` without trailing separators, so two
// spellings of one directory compare equal. Empty on failure.
std::wstring NormalizeDirectory(const std::wstring &path) {
  DWORD size = ::GetFullPathNameW(path.c_str(), 0, nullptr, nullptr);
  if (size == 0) {
    return std::wstring();
  }
  std::vector<wchar_t> buffer(size);
  DWORD length =
      ::GetFullPathNameW(path.c_str(), size, buffer.data(), nullptr);
  if (length == 0 || length >= size) {
    return std::wstring();
  }
  std::wstring full(buffer.data(), length);
  if (!full.empty()) {
    ::CharLowerBuffW(&full[0], static_cast<DWORD>(full.size()));
  }
  while (full.size() > 3 && (full.back() == L'\\' || full.back() == L'/')) {
    full.pop_back();
  }
  return full;
}

// The directory Dart keeps its profile under. Must match configDir() in
// lib/prefs_stub.dart and _localDir() in lib/local/local_offline_io.dart:
// %APPDATA%, or the working directory when APPDATA is unset or empty.
std::wstring DataRoot() {
  DWORD size = ::GetEnvironmentVariableW(L"APPDATA", nullptr, 0);
  if (size > 1) {
    std::vector<wchar_t> buffer(size);
    DWORD length = ::GetEnvironmentVariableW(L"APPDATA", buffer.data(), size);
    if (length > 0 && length < size) {
      return NormalizeDirectory(std::wstring(buffer.data(), length));
    }
  }
  return NormalizeDirectory(L".");
}

std::wstring DefaultDataRoot() {
  PWSTR known = nullptr;
  std::wstring result;
  if (SUCCEEDED(::SHGetKnownFolderPath(FOLDERID_RoamingAppData, 0, nullptr,
                                       &known))) {
    result = NormalizeDirectory(known);
  }
  ::CoTaskMemFree(known);
  return result;
}

// Name of the single-instance mutex. The lock exists so two processes never
// share one store.db, so its scope is the data directory: the normal profile
// keeps the historical name (an older build that is still running uses it, so
// the two still exclude each other), while a process started with APPDATA
// pointed elsewhere -- the release smoke test, scripts/smoke-installer.ps1 --
// gets a name derived from that directory and can run beside the user's Mica.
std::wstring SingleInstanceMutexName() {
#ifdef _DEBUG
  // A debug build keeps its data in the separate mica-dev profile, so it gets
  // its own guard and can run beside the installed app.
  std::wstring name = L"Local\\MicaSingleInstanceDev";
#else
  std::wstring name = L"Local\\MicaSingleInstance";
#endif
  const std::wstring root = DataRoot();
  if (root.empty() || root == DefaultDataRoot()) {
    return name;
  }
  // FNV-1a over the UTF-16 code units; uniqueness, not secrecy, is the point.
  std::uint64_t hash = 14695981039346656037ULL;
  for (wchar_t c : root) {
    hash ^= static_cast<std::uint64_t>(c);
    hash *= 1099511628211ULL;
  }
  wchar_t suffix[17];
  ::swprintf_s(suffix, L"%016llx", static_cast<unsigned long long>(hash));
  return name + L"-" + suffix;
}

}  // namespace

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
  //
  // The mutex is scoped to the data directory; see SingleInstanceMutexName().
  const std::wstring mutex_name = SingleInstanceMutexName();
  HANDLE single_instance_mutex =
      ::CreateMutexW(nullptr, TRUE, mutex_name.c_str());
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
