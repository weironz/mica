//! Outbound email — currently only the password-reset link.
//!
//! A trait object rather than a concrete type so `AppState` (in `app-core`,
//! which must NOT depend on `api-server`) can hold a mailer whose real
//! implementation — the Aliyun DirectMail HTTP client — lives in `api-server`
//! where `reqwest` already is. `app-core` sees only this trait.
//!
//! [`LogMailer`] is the default: it writes the message (reset link included) to
//! the tracing log instead of sending. The whole reset flow is testable before
//! any mail provider is configured — the operator reads the link out of the
//! server logs — and a node that never sets up DirectMail still runs.

use async_trait::async_trait;

/// One outbound message. HTML only: the single email we send (the reset link)
/// is short and wants a clickable button, and every mail client renders HTML.
#[derive(Debug, Clone)]
pub struct Mail {
  pub to: String,
  pub subject: String,
  pub html_body: String,
}

#[async_trait]
pub trait Mailer: Send + Sync {
  /// Send one message. Callers treat a failure as best-effort (the password-reset
  /// endpoint logs and still returns 204, so a mail outage never reveals whether
  /// an address is registered) — so the error is for the log, not the user.
  async fn send(&self, mail: &Mail) -> anyhow::Result<()>;
}

/// The default mailer: log the message instead of sending it. The reset link is
/// in `html_body`, so an operator without DirectMail can still complete a reset
/// by reading it out of the logs — and dev/test never touches the network.
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
  async fn send(&self, mail: &Mail) -> anyhow::Result<()> {
    tracing::info!(
      to = %mail.to,
      subject = %mail.subject,
      body = %mail.html_body,
      "LogMailer: email NOT sent (no mail backend configured) — link is in `body`"
    );
    Ok(())
  }
}
