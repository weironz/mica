//! Cross-platform WebSocket transport for Mica real-time document CRDT sync
//! and sidebar view-tree change notifications.
//!
//! - Native desktop (`#[cfg(not(target_arch = "wasm32"))]`) spawns a lightweight
//!   named OS thread (`mica-ws`) using `tungstenite` (`rustls-tls-webpki-roots`),
//!   passing `Authorization: Bearer <token>` in the HTTP upgrade headers (never
//!   in the URL query string) and waking the NSH async executor via a standard
//!   `Waker` queue without pulling in Tokio.
//! - Web (`#[cfg(target_arch = "wasm32")]`) wraps `web_sys::WebSocket`, relying
//!   on the browser's same-origin `HttpOnly` session cookie (never putting JWTs
//!   in the URL) and cleaning up all JS closures on `close()` / `Drop`.

use uuid::Uuid;

/// Current server WebSocket CRDT sync protocol version (`?v=1`).
pub const WS_PROTOCOL_VERSION: u32 = 1;

/// Events delivered from an active WebSocket connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsEvent {
  Opened,
  Text(String),
  Closed(u16, String),
  Error(String),
}

/// Convert an `http://` or `https://` origin + relative path into a `ws://` or
/// `wss://` WebSocket URL.
pub fn http_to_ws_url(base_origin: &str, path_and_query: &str) -> String {
  let origin = base_origin.trim().trim_end_matches('/');
  let ws_origin = if let Some(rest) = origin.strip_prefix("https://") {
    format!("wss://{rest}")
  } else if let Some(rest) = origin.strip_prefix("http://") {
    format!("ws://{rest}")
  } else if origin.starts_with("wss://") || origin.starts_with("ws://") {
    origin.to_string()
  } else {
    format!("wss://{origin}")
  };
  if path_and_query.starts_with('/') {
    format!("{ws_origin}{path_and_query}")
  } else {
    format!("{ws_origin}/{path_and_query}")
  }
}

/// Build the document CRDT sync WebSocket URL (`/ws/workspaces/{ws}/documents/{doc}?v=1`).
pub fn document_ws_url(base_origin: &str, workspace_id: Uuid, document_id: Uuid) -> String {
  http_to_ws_url(
    base_origin,
    &format!("/ws/workspaces/{workspace_id}/documents/{document_id}?v={WS_PROTOCOL_VERSION}"),
  )
}

/// Build the sidebar views-change WebSocket URL (`/ws/workspaces/{ws}/views`).
pub fn views_ws_url(base_origin: &str, workspace_id: Uuid) -> String {
  http_to_ws_url(base_origin, &format!("/ws/workspaces/{workspace_id}/views"))
}

// ── Native desktop implementation (tungstenite + rustls) ─────────────────────

#[cfg(not(target_arch = "wasm32"))]
mod imp {
  use std::collections::VecDeque;
  use std::future::Future;
  use std::io::ErrorKind;
  use std::net::TcpStream;
  use std::pin::Pin;
  use std::sync::{Arc, Mutex, mpsc};
  use std::task::{Context, Poll, Waker};
  use std::time::Duration;

  use tungstenite::client::IntoClientRequest;
  use tungstenite::http::HeaderValue;
  use tungstenite::stream::MaybeTlsStream;
  use tungstenite::{Error as WsErr, Message, WebSocket};

  use super::WsEvent;

  enum OutboundCmd {
    Text(String),
    Close,
  }

  #[derive(Default)]
  struct QueueState {
    events: VecDeque<WsEvent>,
    waker: Option<Waker>,
    closed: bool,
  }

  impl QueueState {
    fn push(&mut self, ev: WsEvent) {
      self.events.push_back(ev);
      if let Some(w) = self.waker.take() {
        w.wake();
      }
    }

    fn mark_closed(&mut self) {
      self.closed = true;
      if let Some(w) = self.waker.take() {
        w.wake();
      }
    }
  }

  #[derive(Clone)]
  pub struct WsHandle {
    tx: mpsc::Sender<OutboundCmd>,
  }

  impl WsHandle {
    pub fn send_text(&self, payload: String) -> bool {
      self.tx.send(OutboundCmd::Text(payload)).is_ok()
    }

    pub fn close(&self) {
      let _ = self.tx.send(OutboundCmd::Close);
    }
  }

  pub struct WsReceiver {
    queue: Arc<Mutex<QueueState>>,
    tx: mpsc::Sender<OutboundCmd>,
  }

  impl WsReceiver {
    pub async fn recv(&mut self) -> Option<WsEvent> {
      struct RecvFut<'a> {
        queue: &'a Arc<Mutex<QueueState>>,
      }
      impl Future for RecvFut<'_> {
        type Output = Option<WsEvent>;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
          let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
          if let Some(ev) = q.events.pop_front() {
            Poll::Ready(Some(ev))
          } else if q.closed {
            Poll::Ready(None)
          } else {
            q.waker = Some(cx.waker().clone());
            Poll::Pending
          }
        }
      }
      RecvFut { queue: &self.queue }.await
    }
  }

  impl Drop for WsReceiver {
    fn drop(&mut self) {
      let _ = self.tx.send(OutboundCmd::Close);
    }
  }

  fn set_stream_read_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, timeout: Duration) {
    match socket.get_mut() {
      MaybeTlsStream::Plain(tcp) => {
        let _ = tcp.set_read_timeout(Some(timeout));
      }
      MaybeTlsStream::Rustls(tls) => {
        let _ = tls.get_mut().set_read_timeout(Some(timeout));
      }
      _ => {}
    }
  }

  pub fn connect(url: &str, bearer_token: Option<&str>) -> (WsHandle, WsReceiver) {
    let (tx, rx) = mpsc::channel::<OutboundCmd>();
    let queue = Arc::new(Mutex::new(QueueState::default()));
    let q_worker = Arc::clone(&queue);
    let url_owned = url.to_string();
    let token_owned = bearer_token
      .map(str::trim)
      .filter(|t| !t.is_empty())
      .map(str::to_string);

    let _ = std::thread::Builder::new()
      .name("mica-ws".into())
      .spawn(move || {
        let mut req = match url_owned.as_str().into_client_request() {
          Ok(r) => r,
          Err(err) => {
            let mut q = q_worker.lock().unwrap_or_else(|e| e.into_inner());
            q.push(WsEvent::Error(format!("invalid ws url: {err}")));
            q.mark_closed();
            return;
          }
        };
        if let Some(ref tok) = token_owned
          && let Ok(val) = HeaderValue::from_str(&format!("Bearer {tok}"))
        {
          req.headers_mut().insert("Authorization", val);
        }

        let mut socket = match tungstenite::connect(req) {
          Ok((sock, _resp)) => sock,
          Err(err) => {
            let mut q = q_worker.lock().unwrap_or_else(|e| e.into_inner());
            q.push(WsEvent::Error(err.to_string()));
            q.mark_closed();
            return;
          }
        };

        set_stream_read_timeout(&mut socket, Duration::from_millis(20));
        {
          let mut q = q_worker.lock().unwrap_or_else(|e| e.into_inner());
          q.push(WsEvent::Opened);
        }

        let mut close_code = 1000u16;
        let mut close_reason = String::new();
        let mut closed_by_local = false;

        'io: loop {
          // 1. Drain any pending outbound commands before blocking on read.
          loop {
            match rx.try_recv() {
              Ok(OutboundCmd::Text(text)) => {
                if let Err(err) = socket.send(Message::Text(text.into())) {
                  close_code = 1006;
                  close_reason = err.to_string();
                  break 'io;
                }
              }
              Ok(OutboundCmd::Close) => {
                closed_by_local = true;
                let _ = socket.close(None);
                let _ = socket.flush();
                break 'io;
              }
              Err(mpsc::TryRecvError::Empty) => break,
              Err(mpsc::TryRecvError::Disconnected) => {
                closed_by_local = true;
                let _ = socket.close(None);
                break 'io;
              }
            }
          }

          // 2. Read next inbound WebSocket frame (times out every 20ms so
          //    outbound sends never block waiting for server traffic).
          match socket.read() {
            Ok(Message::Text(txt)) => {
              let mut q = q_worker.lock().unwrap_or_else(|e| e.into_inner());
              q.push(WsEvent::Text(txt.to_string()));
            }
            Ok(Message::Ping(payload)) => {
              let _ = socket.send(Message::Pong(payload));
            }
            Ok(Message::Pong(_)) | Ok(Message::Binary(_)) | Ok(Message::Frame(_)) => {}
            Ok(Message::Close(frame)) => {
              if let Some(cf) = frame {
                close_code = cf.code.into();
                close_reason = cf.reason.to_string();
              }
              break 'io;
            }
            Err(WsErr::Io(ref io_err))
              if matches!(
                io_err.kind(),
                ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
              ) =>
            {
              continue;
            }
            Err(WsErr::ConnectionClosed | WsErr::AlreadyClosed) => {
              break 'io;
            }
            Err(err) => {
              close_code = 1006;
              close_reason = err.to_string();
              break 'io;
            }
          }
        }

        let mut q = q_worker.lock().unwrap_or_else(|e| e.into_inner());
        if !closed_by_local {
          q.push(WsEvent::Closed(close_code, close_reason));
        }
        q.mark_closed();
      });

    let handle = WsHandle { tx: tx.clone() };
    let receiver = WsReceiver { queue, tx };
    (handle, receiver)
  }
}

// ── WebAssembly implementation (web_sys::WebSocket) ──────────────────────────

#[cfg(target_arch = "wasm32")]
mod imp {
  use std::cell::RefCell;
  use std::collections::VecDeque;
  use std::future::Future;
  use std::pin::Pin;
  use std::rc::Rc;
  use std::task::{Context, Poll, Waker};

  use wasm_bindgen::JsCast;
  use wasm_bindgen::closure::Closure;
  use web_sys::{CloseEvent, ErrorEvent, MessageEvent, WebSocket};

  use super::WsEvent;

  #[derive(Default)]
  struct QueueState {
    events: VecDeque<WsEvent>,
    waker: Option<Waker>,
    closed: bool,
  }

  impl QueueState {
    fn push(&mut self, ev: WsEvent) {
      self.events.push_back(ev);
      if let Some(w) = self.waker.take() {
        w.wake();
      }
    }

    fn mark_closed(&mut self) {
      self.closed = true;
      if let Some(w) = self.waker.take() {
        w.wake();
      }
    }
  }

  struct WebWsInner {
    ws: Option<WebSocket>,
    _onopen: Option<Closure<dyn FnMut()>>,
    _onmessage: Option<Closure<dyn FnMut(MessageEvent)>>,
    _onclose: Option<Closure<dyn FnMut(CloseEvent)>>,
    _onerror: Option<Closure<dyn FnMut(ErrorEvent)>>,
  }

  impl WebWsInner {
    fn shutdown(&mut self) {
      if let Some(ws) = self.ws.take() {
        ws.set_onopen(None);
        ws.set_onmessage(None);
        ws.set_onclose(None);
        ws.set_onerror(None);
        let _ = ws.close();
      }
      self._onopen = None;
      self._onmessage = None;
      self._onclose = None;
      self._onerror = None;
    }
  }

  #[derive(Clone)]
  pub struct WsHandle {
    inner: Rc<RefCell<WebWsInner>>,
    queue: Rc<RefCell<QueueState>>,
  }

  impl WsHandle {
    pub fn send_text(&self, payload: String) -> bool {
      let inner = self.inner.borrow();
      let Some(ws) = inner.ws.as_ref() else {
        return false;
      };
      if ws.ready_state() != WebSocket::OPEN {
        return false;
      }
      ws.send_with_str(&payload).is_ok()
    }

    pub fn close(&self) {
      self.inner.borrow_mut().shutdown();
      self.queue.borrow_mut().mark_closed();
    }
  }

  pub struct WsReceiver {
    inner: Rc<RefCell<WebWsInner>>,
    queue: Rc<RefCell<QueueState>>,
  }

  impl WsReceiver {
    pub async fn recv(&mut self) -> Option<WsEvent> {
      struct RecvFut<'a> {
        queue: &'a Rc<RefCell<QueueState>>,
      }
      impl Future for RecvFut<'_> {
        type Output = Option<WsEvent>;
        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
          let mut q = self.queue.borrow_mut();
          if let Some(ev) = q.events.pop_front() {
            Poll::Ready(Some(ev))
          } else if q.closed {
            Poll::Ready(None)
          } else {
            q.waker = Some(cx.waker().clone());
            Poll::Pending
          }
        }
      }
      RecvFut { queue: &self.queue }.await
    }
  }

  impl Drop for WsReceiver {
    fn drop(&mut self) {
      self.inner.borrow_mut().shutdown();
      self.queue.borrow_mut().mark_closed();
    }
  }

  pub fn connect(url: &str, _bearer_token: Option<&str>) -> (WsHandle, WsReceiver) {
    let queue = Rc::new(RefCell::new(QueueState::default()));
    let inner = Rc::new(RefCell::new(WebWsInner {
      ws: None,
      _onopen: None,
      _onmessage: None,
      _onclose: None,
      _onerror: None,
    }));

    match WebSocket::new(url) {
      Ok(ws) => {
        let q_open = Rc::clone(&queue);
        let onopen = Closure::<dyn FnMut()>::new(move || {
          q_open.borrow_mut().push(WsEvent::Opened);
        });
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));

        let q_msg = Rc::clone(&queue);
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
          if let Some(text) = ev.data().as_string() {
            q_msg.borrow_mut().push(WsEvent::Text(text));
          }
        });
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));

        let q_close = Rc::clone(&queue);
        let onclose = Closure::<dyn FnMut(CloseEvent)>::new(move |ev: CloseEvent| {
          let mut q = q_close.borrow_mut();
          q.push(WsEvent::Closed(ev.code(), ev.reason()));
          q.mark_closed();
        });
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));

        let q_err = Rc::clone(&queue);
        let onerror = Closure::<dyn FnMut(ErrorEvent)>::new(move |ev: ErrorEvent| {
          let msg = ev.message();
          q_err.borrow_mut().push(WsEvent::Error(if msg.is_empty() {
            "websocket error".into()
          } else {
            msg
          }));
        });
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));

        let mut slot = inner.borrow_mut();
        slot.ws = Some(ws);
        slot._onopen = Some(onopen);
        slot._onmessage = Some(onmessage);
        slot._onclose = Some(onclose);
        slot._onerror = Some(onerror);
      }
      Err(err) => {
        let mut q = queue.borrow_mut();
        q.push(WsEvent::Error(format!("WebSocket::new failed: {err:?}")));
        q.mark_closed();
      }
    }

    let handle = WsHandle {
      inner: Rc::clone(&inner),
      queue: Rc::clone(&queue),
    };
    let receiver = WsReceiver { inner, queue };
    (handle, receiver)
  }
}

pub use imp::{WsHandle, WsReceiver, connect};

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn http_to_ws_url_maps_schemes_cleanly() {
    assert_eq!(
      http_to_ws_url("https://mica.example.com/", "/ws/workspaces/1/views"),
      "wss://mica.example.com/ws/workspaces/1/views"
    );
    assert_eq!(
      http_to_ws_url("http://127.0.0.1:8080", "/ws/workspaces/1/documents/2?v=1"),
      "ws://127.0.0.1:8080/ws/workspaces/1/documents/2?v=1"
    );
    assert_eq!(
      http_to_ws_url("wss://mica.example.com", "ws/test"),
      "wss://mica.example.com/ws/test"
    );
  }

  #[test]
  fn document_and_views_ws_urls_never_embed_tokens() {
    let ws = Uuid::nil();
    let doc = Uuid::nil();
    let doc_url = document_ws_url("https://mica.example.com", ws, doc);
    assert!(doc_url.starts_with("wss://mica.example.com/ws/workspaces/"));
    assert!(doc_url.ends_with("?v=1"));
    assert!(!doc_url.contains("token="));

    let views_url = views_ws_url("http://127.0.0.1:8090", ws);
    assert!(views_url.starts_with("ws://127.0.0.1:8090/ws/workspaces/"));
    assert!(views_url.ends_with("/views"));
    assert!(!views_url.contains("token="));
  }

  #[cfg(not(target_arch = "wasm32"))]
  #[test]
  fn desktop_ws_transport_sends_bearer_header_and_round_trips_frames() {
    use std::future::Future;
    use std::net::TcpListener;
    use std::sync::{Arc, Condvar, Mutex};
    use std::task::{Context, Poll, Wake, Waker};
    use tungstenite::Message;
    use tungstenite::handshake::server::{Request, Response};

    struct CondWaker {
      ready: Mutex<bool>,
      cv: Condvar,
    }
    impl Wake for CondWaker {
      fn wake(self: Arc<Self>) {
        *self.ready.lock().unwrap() = true;
        self.cv.notify_all();
      }
    }
    fn block_on_once<F: Future>(fut: F) -> F::Output {
      let mut fut = Box::pin(fut);
      let wake = Arc::new(CondWaker {
        ready: Mutex::new(false),
        cv: Condvar::new(),
      });
      let waker = Waker::from(Arc::clone(&wake));
      let mut cx = Context::from_waker(&waker);
      loop {
        match fut.as_mut().poll(&mut cx) {
          Poll::Ready(out) => return out,
          Poll::Pending => {
            let mut g = wake.ready.lock().unwrap();
            while !*g {
              g = wake.cv.wait(g).unwrap();
            }
            *g = false;
          }
        }
      }
    }

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ws test listener");
    let addr = listener.local_addr().unwrap();
    let seen_auth = Arc::new(Mutex::new(String::new()));
    let seen_auth_srv = Arc::clone(&seen_auth);

    let srv = std::thread::spawn(move || {
      let (stream, _) = listener.accept().unwrap();
      #[allow(clippy::result_large_err)]
      let mut ws = tungstenite::accept_hdr(stream, |req: &Request, resp: Response| {
        if let Some(h) = req
          .headers()
          .get("authorization")
          .and_then(|v| v.to_str().ok())
        {
          *seen_auth_srv.lock().unwrap() = h.to_string();
        }
        Ok(resp)
      })
      .unwrap();
      if let Ok(Message::Text(txt)) = ws.read() {
        let _ = ws.send(Message::Text(format!("echo:{txt}").into()));
      }
      let _ = ws.close(None);
    });

    let (handle, mut rx) = connect(&format!("ws://{addr}/ws/test"), Some("secret-jwt"));
    assert_eq!(block_on_once(rx.recv()), Some(WsEvent::Opened));
    assert!(handle.send_text(r#"{"type":"ping"}"#.into()));
    assert_eq!(
      block_on_once(rx.recv()),
      Some(WsEvent::Text(r#"echo:{"type":"ping"}"#.into()))
    );
    handle.close();
    srv.join().unwrap();
    assert_eq!(seen_auth.lock().unwrap().as_str(), "Bearer secret-jwt");
  }
}
