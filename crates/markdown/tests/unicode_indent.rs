use mica_markdown::import_markdown;
use std::{
  process::{Command, Stdio},
  thread,
  time::{Duration, Instant},
};

#[test]
fn unicode_indentation_finishes_and_keeps_list_code() {
  const CHILD: &str = "MICA_UNICODE_INDENT_CHILD";
  if std::env::var_os(CHILD).is_some() {
    for whitespace in [
      ' ', '\t', '\u{a0}', '\u{1680}', '\u{2003}', '\u{202f}', '\u{205f}', '\u{3000}',
    ] {
      let input = format!("- a\n{}b", whitespace.to_string().repeat(6));
      let snapshot = import_markdown(&input, "root");
      let code = snapshot
        .blocks
        .iter()
        .find(|b| b.kind == "code_block")
        .expect("indented list code");
      assert!(code.text.ends_with('b'), "{whitespace:?}: {code:?}");
    }
    return;
  }
  let mut child = Command::new(std::env::current_exe().unwrap())
    .args([
      "--exact",
      "unicode_indentation_finishes_and_keeps_list_code",
      "--nocapture",
    ])
    .env(CHILD, "1")
    .stdout(Stdio::null())
    .stderr(Stdio::inherit())
    .spawn()
    .unwrap();
  let started = Instant::now();
  loop {
    if let Some(status) = child.try_wait().unwrap() {
      assert!(
        status.success(),
        "Unicode indentation child failed: {status}"
      );
      break;
    }
    // 60s, not 2s: the watchdog is WALL time and sibling test processes on
    // this machine can starve the child for seconds — a pre-fix infinite
    // loop runs forever, so a loose bound still catches the regression.
    if started.elapsed() > Duration::from_secs(60) {
      child.kill().unwrap();
      child.wait().unwrap();
      panic!("Unicode indentation did not finish within 60 seconds");
    }
    thread::sleep(Duration::from_millis(10));
  }
}
