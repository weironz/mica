#include "flutter_window.h"

#include <optional>

#include "flutter/generated_plugin_registrant.h"

namespace {
// The installer smoke test opts in to a first-frame receipt. A live process
// alone cannot distinguish a working editor from a stalled/late-crashing boot.
void WriteFirstFrameReceipt() {
  wchar_t path[MAX_PATH];
  const DWORD length = ::GetEnvironmentVariableW(
      L"MICA_STARTUP_PROBE_FILE", path, MAX_PATH);
  if (length == 0 || length >= MAX_PATH) return;
  HANDLE receipt = ::CreateFileW(path, GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS,
                                FILE_ATTRIBUTE_NORMAL, nullptr);
  if (receipt == INVALID_HANDLE_VALUE) return;
  constexpr char marker[] = "first_frame\n";
  DWORD written = 0;
  ::WriteFile(receipt, marker, sizeof(marker) - 1, &written, nullptr);
  ::CloseHandle(receipt);
}
}  // namespace

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());
  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    first_frame_ready_ = true;
    this->Show();
    // gh#190: window_manager may show the window and apply TitleBarStyle.hidden
    // (SWP_NOSIZE | SWP_FRAMECHANGED) before the first Flutter frame arrives,
    // leaving ANGLE stuck on a white surface until a manual resize. Re-sync the
    // child HWND and force a surface refresh once the first frame is ready.
    RefreshSurface(true);
    WriteFirstFrameReceipt();
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  return true;
}

void FlutterWindow::RefreshSurface(bool force_surface) {
  if (!flutter_controller_ || GetHandle() == nullptr ||
      ::IsIconic(GetHandle())) {
    return;
  }
  if (force_surface) {
    NudgeChildContentSurface();
  } else {
    SyncChildContentBounds();
  }
  flutter_controller_->ForceRedraw();
}

void FlutterWindow::OnDestroy() {
  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_FONTCHANGE:
      if (flutter_controller_) {
        flutter_controller_->engine()->ReloadSystemFonts();
      }
      break;
    case WM_SHOWWINDOW:
      // Refresh after un-hiding from the tray or single-instance activation.
      if (wparam == TRUE && first_frame_ready_) {
        const LRESULT handled =
            Win32Window::MessageHandler(hwnd, message, wparam, lparam);
        RefreshSurface(true);
        return handled;
      }
      break;
    case WM_DISPLAYCHANGE:
      // Refresh after monitor/virtual-display/RDP topology changes.
      if (first_frame_ready_) {
        const LRESULT handled =
            Win32Window::MessageHandler(hwnd, message, wparam, lparam);
        RefreshSurface(true);
        return handled;
      }
      break;
    case WM_WINDOWPOSCHANGED:
      // TitleBarStyle.hidden sends SWP_NOSIZE | SWP_FRAMECHANGED, which alters
      // the client area without emitting WM_SIZE.
      if (lparam != 0 &&
          (reinterpret_cast<const WINDOWPOS*>(lparam)->flags &
           SWP_FRAMECHANGED) != 0) {
        const LRESULT handled =
            Win32Window::MessageHandler(hwnd, message, wparam, lparam);
        RefreshSurface(first_frame_ready_);
        return handled;
      }
      break;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
