#ifndef RUNNER_FLUTTER_WINDOW_H_
#define RUNNER_FLUTTER_WINDOW_H_

#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>

#include <memory>

#include "win32_window.h"

// A window that does nothing but host a Flutter view.
class FlutterWindow : public Win32Window {
 public:
  // Creates a new FlutterWindow hosting a Flutter view running |project|.
  explicit FlutterWindow(const flutter::DartProject& project);
  virtual ~FlutterWindow();

 protected:
  // Win32Window:
  bool OnCreate() override;
  void OnDestroy() override;
  LRESULT MessageHandler(HWND window, UINT const message, WPARAM const wparam,
                         LPARAM const lparam) noexcept override;

 private:
  // Align the Flutter child view with the client area and ask the engine to
  // present a fresh frame. Pass force_surface=true on show/display-change/first
  // frame so ANGLE/D3D11 cannot stay stuck on a blank white surface (gh#190).
  void RefreshSurface(bool force_surface);

  // The project to run.
  flutter::DartProject project_;

  // The Flutter instance hosted by this window.
  std::unique_ptr<flutter::FlutterViewController> flutter_controller_;

  // Guards against triggering a surface nudge before the first Flutter frame.
  bool first_frame_ready_ = false;
};

#endif  // RUNNER_FLUTTER_WINDOW_H_
