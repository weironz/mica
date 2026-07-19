//! Single-page PDF export (Path α): render a self-contained HTML string to a
//! REAL PDF — vector, selectable text, embedded CJK font subsets — via the
//! OS-provided WebView2 runtime's headless print-to-PDF.
//!
//! Why this shape (see docs / memory `export-html-pdf-word`): the expensive 90%
//! of PDF (text shaping, font fallback, subsetting, CJK, pagination) is done by
//! the browser we already ship on Windows 11 (WebView2 is preinstalled). We do
//! NOT bundle a browser and do NOT re-implement layout — we reuse the same
//! self-contained HTML the app already exports, hand it to the runtime, and get
//! back the bytes. HTML generation stays in Rust (`export_html`); this is the
//! one platform-glue step that turns those bytes into PDF bytes.
//!
//! Windows-only for now: web uses `window.print()`, macOS/Linux come later
//! (`WKWebView.createPDF` / WebKitGTK). On non-Windows this returns `None`.

/// Render `html` (a complete, self-contained HTML document) to PDF bytes, or
/// `None` if unsupported on this platform / the runtime is unavailable / print
/// failed. Async by default (FRB runs it off the Dart isolate); internally it
/// hosts an off-screen WebView2 on a dedicated STA thread.
pub fn export_pdf(html: String) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        win::run(html)
    }
    #[cfg(not(windows))]
    {
        let _ = html;
        None
    }
}

#[cfg(windows)]
mod win {
    use std::sync::mpsc;

    use std::time::Duration;

    use webview2_com::{
        CoreWebView2EnvironmentOptions,
        Microsoft::Web::WebView2::Win32::{
            CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2, ICoreWebView2Controller,
            ICoreWebView2Environment, ICoreWebView2EnvironmentOptions, ICoreWebView2_7,
        },
        CreateCoreWebView2ControllerCompletedHandler,
        CreateCoreWebView2EnvironmentCompletedHandler, NavigationCompletedEventHandler,
        PrintToPdfCompletedHandler,
    };
    use windows::{
        core::{w, Interface, HSTRING, PCWSTR},
        Win32::{
            Foundation::{E_POINTER, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM},
            System::{
                Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
                LibraryLoader::GetModuleHandleW,
                Threading::GetCurrentThreadId,
            },
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, PostThreadMessageW,
                RegisterClassW, CW_USEDEFAULT, WM_QUIT, WNDCLASSW, WS_OVERLAPPEDWINDOW,
            },
        },
    };

    /// Deadline for one export. Rendering + printing even a huge, image-heavy
    /// document finishes in seconds; minutes means the WebView2 (or the GPU
    /// under it) is wedged — exactly the machine state this watchdog exists
    /// for. Without it every async wait below blocks in GetMessage forever,
    /// the FRB worker blocks in join(), the msedgewebview2 process tree
    /// lingers, and each retry stacks another stuck thread.
    const EXPORT_DEADLINE: Duration = Duration::from_secs(120);

    /// After waking the pump with WM_QUIT, how long to let the STA thread
    /// unwind before abandoning (detaching) it.
    const QUIT_GRACE: Duration = Duration::from_secs(10);

    /// Run the whole thing on a FRESH thread: WebView2 needs a COM STA apartment
    /// with a message pump, but the FRB worker thread we're invoked on may
    /// already be COM-MTA (an STA init there would fail with RPC_E_CHANGED_MODE).
    /// A brand-new thread has no apartment yet, so STA init always succeeds.
    ///
    /// The wait is bounded (see [EXPORT_DEADLINE]): on timeout we post WM_QUIT
    /// at the STA thread — the crate's `wait_with_pump` blocks in GetMessage,
    /// which returns 0 on WM_QUIT and maps to `Error::TaskCanceled`, so the
    /// thread unwinds through the normal error path (whose guards close the
    /// controller and destroy the host window). If it STILL doesn't come back
    /// (stuck inside a synchronous COM/GPU call), we abandon the thread rather
    /// than hang the app with it.
    ///
    /// `pub(super)`, not `pub`: only `export_pdf` calls it, and a fully-public
    /// fn would make flutter_rust_bridge generate a (broken) binding into this
    /// private module.
    pub(super) fn run(html: String) -> Option<Vec<u8>> {
        let (tid_tx, tid_rx) = mpsc::channel();
        let (res_tx, res_rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _ = tid_tx.send(unsafe { GetCurrentThreadId() });
            let _ = res_tx.send(html_to_pdf(&html));
        });
        let Ok(tid) = tid_rx.recv() else {
            return None; // the thread died before it even reported its id
        };
        let outcome = res_rx.recv_timeout(EXPORT_DEADLINE).or_else(|e| {
            // Disconnected = the worker DIED (panicked) rather than hung:
            // nothing to wake — and its thread id may already be recycled to
            // an unrelated thread, so posting WM_QUIT at it would be wrong.
            if matches!(e, mpsc::RecvTimeoutError::Disconnected) {
                return Err(e);
            }
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            res_rx.recv_timeout(QUIT_GRACE)
        });
        match outcome {
            Ok(result) => {
                let _ = handle.join();
                match result {
                    Ok(bytes) => Some(bytes),
                    Err(e) => {
                        eprintln!("[export_pdf] failed: {e}");
                        None
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("[export_pdf] worker thread died (panic?)");
                let _ = handle.join(); // already dead — reap, don't hang
                None
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                eprintln!(
                    "[export_pdf] timed out after {EXPORT_DEADLINE:?}; \
                     abandoning the WebView2 thread (wedged runtime/GPU)"
                );
                drop(handle); // detach — never hang the app on a dead browser
                None
            }
        }
    }

    fn html_to_pdf(html: &str) -> Result<Vec<u8>, String> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| format!("CoInitializeEx: {e}"))?;
        }
        let out = render(html);
        unsafe { CoUninitialize() };
        out
    }

    fn render(html: &str) -> Result<Vec<u8>, String> {
        // Stage the HTML as a temp file and navigate to it via file:// rather
        // than NavigateToString: our exported HTML inlines images as data URIs,
        // which can blow past NavigateToString's in-memory string cap.
        let dir = std::env::temp_dir();
        let stem = format!("mica_pdf_{}", uuid::Uuid::new_v4());
        let html_path = dir.join(format!("{stem}.html"));
        let pdf_path = dir.join(format!("{stem}.pdf"));
        std::fs::write(&html_path, html).map_err(|e| format!("write html: {e}"))?;

        let render_res = render_inner(&html_path, &pdf_path);
        let _ = std::fs::remove_file(&html_path);

        let bytes = match render_res {
            Ok(()) => std::fs::read(&pdf_path).map_err(|e| format!("read pdf: {e}")),
            Err(e) => Err(e),
        };
        let _ = std::fs::remove_file(&pdf_path);
        bytes
    }

    fn render_inner(html_path: &std::path::Path, pdf_path: &std::path::Path) -> Result<(), String> {
        let hwnd = create_host_window().map_err(|e| format!("host window: {e}"))?;
        // Everything past the window lives in closures so the controller and
        // window are torn down on EVERY path — the old `?` early-returns
        // skipped Close/DestroyWindow on failure, and the watchdog's WM_QUIT
        // unwind (run(), TaskCanceled) arrives precisely through those paths.
        let result = (|| {
            let environment = create_environment().map_err(|e| format!("environment: {e}"))?;
            let controller =
                create_controller(&environment, hwnd).map_err(|e| format!("controller: {e}"))?;
            let body = (|| {
                unsafe {
                    let _ = controller.SetBounds(RECT {
                        left: 0,
                        top: 0,
                        right: 1024,
                        bottom: 1400,
                    });
                    let _ = controller.SetIsVisible(false);
                }
                let webview = unsafe { controller.CoreWebView2() }
                    .map_err(|e| format!("CoreWebView2: {e}"))?;

                let url =
                    format!("file:///{}", html_path.to_string_lossy().replace('\\', "/"));
                navigate_and_wait(&webview, &url)?;

                // PrintToPdf lives on ICoreWebView2_7 (Runtime ≥ 1.0.1054); the
                // cast fails only on an ancient runtime — surfaced as an error.
                let webview7: ICoreWebView2_7 = webview
                    .cast()
                    .map_err(|e| format!("ICoreWebView2_7 (WebView2 runtime too old?): {e}"))?;
                print_to_pdf(&webview7, pdf_path)
            })();
            unsafe {
                let _ = controller.Close();
            }
            body
        })();
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
        result
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        DefWindowProcW(hwnd, msg, w, l)
    }

    fn create_host_window() -> windows::core::Result<HWND> {
        unsafe {
            let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                lpszClassName: w!("MicaPdfHost"),
                hInstance: hinstance,
                ..Default::default()
            };
            // Idempotent: a second in-process call returns 0 (already
            // registered), which is fine — the class persists process-wide.
            RegisterClassW(&class);
            // Never shown (no WS_VISIBLE, no ShowWindow): it only exists to host
            // the off-screen WebView2 controller.
            CreateWindowExW(
                Default::default(),
                w!("MicaPdfHost"),
                w!("MicaPdfHost"),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1024,
                1400,
                None,
                None,
                Some(hinstance),
                None,
            )
        }
    }

    /// Dedicated user-data folder for the print WebView2. Without one, the
    /// runtime defaults to `<Exe>.WebView2` NEXT TO the executable — under
    /// Program Files that's not reliably writable, and it quietly accumulates
    /// a browser/shader cache beside the install. %LOCALAPPDATA% is the
    /// documented right place for it.
    fn user_data_folder() -> std::path::PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("mica")
            .join("pdf_webview2")
    }

    fn create_environment() -> Result<ICoreWebView2Environment, String> {
        let udf = user_data_folder();
        let _ = std::fs::create_dir_all(&udf);
        let udf_h = HSTRING::from(udf.to_string_lossy().as_ref());
        let (tx, rx) = mpsc::channel();
        // `wait_for_async_operation` pumps messages until the completed closure
        // runs, so the value is in `rx` by the time it returns.
        CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                // Headless print-to-PDF needs no GPU: `--disable-gpu` keeps this
                // transient browser off the D3D device and the DWM compositor
                // entirely (its GPU child process is otherwise a real client of
                // the same compositor the machine's WATCHDOG freezes point at).
                // It applies on every export because each export starts the
                // browser process fresh under our dedicated user-data folder.
                let opts = CoreWebView2EnvironmentOptions::default();
                opts.set_additional_browser_arguments(
                    "--disable-gpu --disable-gpu-compositing".to_string(),
                );
                let options: ICoreWebView2EnvironmentOptions = opts.into();
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(), // browser exe folder: the installed runtime
                    &udf_h,
                    &options,
                    &handler,
                )
                .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |error_code, environment| {
                error_code?;
                let _ = tx.send(environment.ok_or_else(|| windows::core::Error::from(E_POINTER)));
                Ok(())
            }),
        )
        .map_err(|e| e.to_string())?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }

    fn create_controller(
        environment: &ICoreWebView2Environment,
        hwnd: HWND,
    ) -> Result<ICoreWebView2Controller, String> {
        let (tx, rx) = mpsc::channel();
        let env = environment.clone();
        CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                env.CreateCoreWebView2Controller(hwnd, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |error_code, controller| {
                error_code?;
                let _ = tx.send(controller.ok_or_else(|| windows::core::Error::from(E_POINTER)));
                Ok(())
            }),
        )
        .map_err(|e| e.to_string())?;
        rx.recv()
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())
    }

    fn navigate_and_wait(webview: &ICoreWebView2, url: &str) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let handler = NavigationCompletedEventHandler::create(Box::new(move |_sender, _args| {
            let _ = tx.send(());
            Ok(())
        }));
        let mut token = 0;
        unsafe {
            webview
                .add_NavigationCompleted(&handler, &mut token)
                .map_err(|e| format!("add_NavigationCompleted: {e}"))?;
            let url_h = HSTRING::from(url);
            webview
                .Navigate(PCWSTR(url_h.as_ptr()))
                .map_err(|e| format!("Navigate: {e}"))?;
        }
        let waited = webview2_com::wait_with_pump(rx).map_err(|e| format!("navigation wait: {e}"));
        unsafe {
            let _ = webview.remove_NavigationCompleted(token);
        }
        waited
    }

    fn print_to_pdf(webview7: &ICoreWebView2_7, pdf_path: &std::path::Path) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let path_h = HSTRING::from(pdf_path.to_string_lossy().as_ref());
        let wv = webview7.clone();
        PrintToPdfCompletedHandler::wait_for_async_operation(
            Box::new(move |handler| unsafe {
                wv.PrintToPdf(PCWSTR(path_h.as_ptr()), None, &handler)
                    .map_err(webview2_com::Error::WindowsError)
            }),
            Box::new(move |error_code, is_successful| {
                error_code?;
                let _ = tx.send(is_successful);
                Ok(())
            }),
        )
        .map_err(|e| e.to_string())?;
        match rx.recv() {
            Ok(true) => Ok(()),
            Ok(false) => Err("PrintToPdf reported failure".into()),
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    // Exercises the real WebView2 runtime on the dev machine: HTML in, real
    // vector PDF out. Skips gracefully if the runtime is somehow unavailable.
    #[test]
    fn exports_a_real_vector_pdf() {
        let html = "<!doctype html><html lang=\"zh\"><head><meta charset=\"utf-8\">\
            <title>t</title></head><body><h1>导出验证</h1>\
            <p>中文正文 with <b>bold</b> and <code>code</code>.</p>\
            <table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td>2</td></tr></table>\
            </body></html>"
            .to_string();
        let Some(bytes) = export_pdf(html) else {
            eprintln!("WebView2 runtime unavailable — skipping");
            return;
        };
        assert!(bytes.starts_with(b"%PDF"), "not a PDF: {:?}", &bytes[..8.min(bytes.len())]);
        assert!(bytes.len() > 1000, "PDF suspiciously small: {} bytes", bytes.len());
        // Vector text with embedded fonts, not a rasterized page image.
        let hay = bytes.as_slice();
        assert!(
            hay.windows(10).any(|w| w == b"/FontFile2"),
            "no embedded font subset — expected vector text"
        );
    }
}
