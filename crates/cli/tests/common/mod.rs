//! A loopback fake Mica API for BINARY-level CLI tests (#112, #118).
//!
//! These tests run the real `mica-cli` executable against a real TCP socket:
//! the exit code and the stdout/stderr split are the contract, so nothing short
//! of spawning the binary can observe them. The handler gets the request line
//! and returns the whole response.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// What the fake server answers with.
pub struct Reply {
  pub status: u16,
  pub content_type: &'static str,
  pub body: String,
}

// Each integration-test target compiles this module as its own crate; helpers
// used only by the OTHER test then read as dead to clippy. Keep both sites of
// the fake API together and tolerate that split.
#[allow(dead_code)]
impl Reply {
  pub fn json(status: u16, body: impl Into<String>) -> Self {
    Reply {
      status,
      content_type: "application/json",
      body: body.into(),
    }
  }

  pub fn text(status: u16, body: impl Into<String>) -> Self {
    Reply {
      status,
      content_type: "text/plain",
      body: body.into(),
    }
  }
}

#[allow(dead_code)] // see the `impl Reply` note above — per-target split
pub struct FakeServer {
  base: String,
  seen: Arc<Mutex<Vec<String>>>,
}

// Same split as `impl Reply`: `base` is used by both tests, `requests` only by
// the other one — dead_code fires per compiled target otherwise.
#[allow(dead_code)]
impl FakeServer {
  /// Serve until the test process exits. [handler] is called with
  /// `"METHOD /api/path?query"` and chooses the reply.
  pub fn start(handler: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Self {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake API");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
      for stream in listener.incoming() {
        let Ok(stream) = stream else { return };
        let handler = handler.clone();
        let seen = log.clone();
        std::thread::spawn(move || {
          let mut stream = stream;
          if let Some(request) = read_request(&mut stream) {
            seen.lock().unwrap().push(request.clone());
            let reply = handler(&request);
            // The stub never keeps a connection alive: every response is
            // complete and terminated, exactly like the real API behind a proxy.
            let _ = write_reply(stream, reply);
          }
        });
      }
    });
    FakeServer { base, seen }
  }

  pub fn base(&self) -> &str {
    &self.base
  }

  pub fn requests(&self) -> Vec<String> {
    self.seen.lock().unwrap().clone()
  }
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
  let mut reader = BufReader::new(&mut *stream);
  let mut request_line = String::new();
  if reader.read_line(&mut request_line).ok()? == 0 {
    return None;
  }
  let mut content_length = 0usize;
  loop {
    let mut header = String::new();
    if reader.read_line(&mut header).ok()? == 0 || header == "\r\n" {
      break;
    }
    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:") {
      content_length = value.trim().parse().unwrap_or(0);
    }
  }
  let mut body = vec![0u8; content_length];
  let _ = reader.read_exact(&mut body);
  Some(request_line.trim_end().to_string())
}

fn write_reply(mut stream: TcpStream, reply: Reply) -> std::io::Result<()> {
  write!(
    stream,
    "HTTP/1.1 {} X\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
    reply.status,
    reply.content_type,
    reply.body.len()
  )?;
  stream.write_all(reply.body.as_bytes())?;
  stream.flush()
}

/// A private config path so the developer's real `auth login` cannot leak into
/// the test. The file does not exist: `Config::load` treats that as defaults.
pub fn isolated_config() -> std::path::PathBuf {
  let dir = std::env::temp_dir().join(format!(
    "mica-cli-it-{}-{:?}",
    std::process::id(),
    std::thread::current().id()
  ));
  std::fs::create_dir_all(&dir).expect("create test config dir");
  dir.join("config.json")
}

/// Write [contents] to a fresh temp file and return its path.
pub fn temp_file(name: &str, contents: &str) -> std::path::PathBuf {
  let dir = std::env::temp_dir().join(format!("mica-cli-it-{}", std::process::id()));
  std::fs::create_dir_all(&dir).expect("create temp dir");
  let path = dir.join(name);
  std::fs::write(&path, contents).expect("write temp file");
  path
}

pub struct Run {
  pub status: i32,
  pub stdout: String,
  pub stderr: String,
}

/// Run the freshly built binary with the loopback server and an isolated config.
pub fn run_cli(server: &FakeServer, args: &[&str]) -> Run {
  let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_mica-cli"));
  command
    .arg("--server")
    .arg(server.base())
    .arg("--token")
    .arg("dummy")
    .env("MICA_CONFIG", isolated_config())
    .env_remove("MICA_SERVER")
    .env_remove("MICA_PAT")
    .env_remove("MICA_TOKEN")
    .args(args);
  let output = command.output().expect("spawn mica-cli");
  Run {
    status: output.status.code().expect("binary exited with a signal"),
    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
  }
}
