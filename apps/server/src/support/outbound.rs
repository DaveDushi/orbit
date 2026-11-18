//! `support.send`: sends a queued reply with the SMTP server of its mailbox (not the instance's:
//! customer mail of a workspace leaves from its own domain, so SPF and DKIM stay correct). Network
//! errors and temporary SMTP answers retry with backoff; the last failure, a permanent SMTP
//! answer or the mailbox's daily cap marks the reply "failed", and a member can send it again.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lettre::message::Mailbox;
use lettre::message::header::ContentType;
use lettre::{Address, AsyncTransport, Message};
use orbit_platform::{
    Database, Id, JobError, JobKind, JobKindRegistrationError, ThrottleDecision, TimestampMillis,
    Worker,
};
use serde_json::Value;

use crate::email_quota::{self, Quota};
use crate::repositories::instance_settings::{SmtpSecurity, SmtpSettings};
use crate::repositories::support::{Outgoing, SupportRepository, smtp_host_allowed};
use crate::support::inbound::{one_line, valid_address};

pub const SEND_JOB: &str = "support.send";
/// Replies one mailbox may send in 24 hours (counted in `email_sends`).
pub const DAILY_SENDS: u32 = 500;

#[must_use]
pub fn job_kind() -> JobKind {
    JobKind::new(SEND_JOB).with_retry_policy(
        5,
        vec![
            Duration::from_secs(30),
            Duration::from_secs(2 * 60),
            Duration::from_secs(10 * 60),
            Duration::from_secs(30 * 60),
        ],
    )
}

/// Messages a recording sender kept instead of sending them (tests).
pub type SentMail = Arc<Mutex<Vec<Vec<u8>>>>;

/// How replies leave: SMTP, or for tests a list.
#[derive(Clone)]
pub struct SupportSender {
    database: Database,
    app_key: Option<[u8; 32]>,
    development: bool,
    recorded: Option<SentMail>,
}

/// A failed attempt; `retry` when a later attempt may work.
#[derive(Debug)]
pub struct SendFailure {
    pub text: String,
    pub retry: bool,
}

impl SendFailure {
    fn permanent(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            retry: false,
        }
    }
}

impl SupportSender {
    #[must_use]
    pub fn new(database: Database, app_key: Option<[u8; 32]>, development: bool) -> Self {
        Self {
            database,
            app_key,
            development,
            recorded: None,
        }
    }

    /// Keeps the formatted messages in the returned list instead of sending them.
    #[must_use]
    pub fn recording(database: Database, app_key: [u8; 32]) -> (Self, SentMail) {
        let sent = SentMail::default();
        let sender = Self {
            database,
            app_key: Some(app_key),
            development: true,
            recorded: Some(Arc::clone(&sent)),
        };
        (sender, sent)
    }

    /// One attempt to send reply `message_id`, and its result recorded on the reply. `last`: no
    /// retry follows a failure.
    pub async fn deliver(&self, message_id: Id, last: bool) -> Result<(), SendFailure> {
        let repository = SupportRepository::new(self.database.clone());
        let outgoing = repository
            .outgoing(message_id)
            .await
            .map_err(|_| SendFailure {
                text: "The reply could not be read.".to_owned(),
                retry: true,
            })?;
        // Sent, failed for good, or deleted meanwhile.
        let Some(outgoing) = outgoing else {
            return Ok(());
        };
        let result = self.send(&outgoing).await;
        let recorded = match &result {
            Ok(()) => Ok(()),
            Err(failure) => Err((failure.text.clone(), last || !failure.retry)),
        };
        repository
            .finish_send(message_id, recorded)
            .await
            .map_err(|_| SendFailure {
                text: "The send result could not be saved.".to_owned(),
                retry: true,
            })?;
        result
    }

    async fn send(&self, outgoing: &Outgoing) -> Result<(), SendFailure> {
        let message = build_message(outgoing)?;
        let decision = email_quota::take(
            &self.database,
            &[Quota {
                scope: "support_mailbox",
                subject: outgoing.mailbox_id.to_string(),
                per_day: DAILY_SENDS,
                cooldown: Duration::ZERO,
            }],
            TimestampMillis::now(),
        )
        .await
        .map_err(|_| SendFailure {
            text: "The send limit could not be checked.".to_owned(),
            retry: true,
        })?;
        if let ThrottleDecision::RetryAfter(_) = decision {
            return Err(SendFailure::permanent(
                "The mailbox reached its limit of 500 emails in 24 hours.",
            ));
        }
        if let Some(sent) = &self.recorded {
            sent.lock()
                .expect("sent mail lock")
                .push(message.formatted());
            return Ok(());
        }
        let settings = smtp_settings(outgoing)?;
        if !self.development {
            check_resolved_host(&settings.host, settings.port).await?;
        }
        let password = self.password(&settings);
        let transport =
            crate::mail::transport(&settings, self.app_key.as_ref()).map_err(|error| {
                SendFailure::permanent(&scrub(&error.to_string(), password.as_deref()))
            })?;
        transport
            .send(message)
            .await
            .map(|_| ())
            .map_err(|error| SendFailure {
                // The server's answer, never the message or the password.
                text: explain(&error.to_string(), password.as_deref()),
                retry: !error.is_permanent(),
            })
    }

    /// The saved SMTP password in plain text, only to remove it from error texts.
    fn password(&self, settings: &SmtpSettings) -> Option<String> {
        let sealed = settings.password.as_ref()?;
        crate::secret_box::decrypt_secret(self.app_key.as_ref()?, sealed).ok()
    }

    /// Signs in to the mailbox's SMTP server and quits, without sending: the "Test connection"
    /// of the mailbox settings. `Err` is the text to show (never containing the password).
    pub async fn test_connection(&self, settings: &SmtpSettings) -> Result<(), String> {
        if !self.development {
            check_resolved_host(&settings.host, settings.port)
                .await
                .map_err(|failure| failure.text)?;
        }
        let password = self.password(settings);
        let transport = crate::mail::transport(settings, self.app_key.as_ref())
            .map_err(|error| scrub(&error.to_string(), password.as_deref()))?;
        match transport.test_connection().await {
            Ok(true) => Ok(()),
            Ok(false) => Err("The SMTP server closed the connection.".to_owned()),
            Err(error) => Err(explain(&error.to_string(), password.as_deref())),
        }
    }
}

/// Removes the password (and its base64 forms, as AUTH sends it) from a text.
fn scrub(text: &str, password: Option<&str>) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    let Some(password) = password.filter(|password| !password.is_empty()) else {
        return text.to_owned();
    };
    [password.to_owned(), STANDARD.encode(password)]
        .iter()
        .fold(text.to_owned(), |text, secret| {
            text.replace(secret.as_str(), "•••")
        })
}

/// What an SMTP failure means, with the fix for the common Microsoft 365 answers.
pub(crate) fn explain(error: &str, password: Option<&str>) -> String {
    let error = scrub(error, password);
    if error.contains("5.7.139") {
        return format!(
            "Microsoft 365 refused the sign-in: SMTP AUTH is off for this mailbox or the whole tenant (535 5.7.139). \
             Fix: Microsoft 365 admin center → Users → Active users → the mailbox → Mail → Manage email apps → turn on \
             Authenticated SMTP; if the error says \"for the Tenant\", Exchange admin center → Settings → Mail flow → \
             clear \"Turn off SMTP AUTH protocol for your organization\". Changes take up to an hour, then Test connection \
             again. Server answer: {error}"
        );
    }
    if error.contains("5.7.30") {
        return format!(
            "Microsoft 365 no longer accepts a password for SMTP in this tenant (550 5.7.30, Basic authentication is \
             off). An admin can allow it again for now in Exchange Online; see docs/support.md. Server answer: {error}"
        );
    }
    if error.contains("535") {
        return format!(
            "The SMTP server refused the user name or password (535). Check the SMTP user name (for Microsoft 365: the \
             full mailbox address) and the password. Server answer: {error}"
        );
    }
    format!("The mail server refused: {error}")
}

fn smtp_settings(outgoing: &Outgoing) -> Result<SmtpSettings, SendFailure> {
    let missing = || SendFailure::permanent("The mailbox has no SMTP server.");
    Ok(SmtpSettings {
        host: outgoing.smtp_host.clone().ok_or_else(missing)?,
        port: outgoing.smtp_port.ok_or_else(missing)?,
        security: outgoing
            .smtp_security
            .as_deref()
            .and_then(SmtpSecurity::parse)
            .ok_or_else(missing)?,
        username: outgoing.smtp_username.clone(),
        password: outgoing.smtp_password.clone(),
        from_address: outgoing.from_address.clone(),
        from_name: None,
    })
}

/// The host must still resolve to public addresses only (a name can change after it was saved).
async fn check_resolved_host(host: &str, port: u16) -> Result<(), SendFailure> {
    if !smtp_host_allowed(host, port, false) {
        return Err(SendFailure::permanent("The SMTP server is not allowed."));
    }
    let addresses: Vec<_> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| SendFailure {
            text: "The SMTP server's name could not be resolved.".to_owned(),
            retry: true,
        })?
        .collect();
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !orbit_platform::is_public_ip(address.ip()))
    {
        return Err(SendFailure::permanent(
            "The SMTP server resolves to a private address.",
        ));
    }
    Ok(())
}

/// The reply as plain text, with the threading headers. Addresses, names and the subject were
/// checked when the reply was queued; they are checked again here, so no line break can add a
/// header.
pub(crate) fn build_message(outgoing: &Outgoing) -> Result<Message, SendFailure> {
    let invalid =
        |field: &str| SendFailure::permanent(&format!("The reply has an invalid {field}."));
    if !valid_address(&outgoing.from_address) {
        return Err(invalid("sender address"));
    }
    if !valid_address(&outgoing.to) {
        return Err(invalid("recipient address"));
    }
    let header_ok = |value: &str| !value.contains(['\r', '\n']) && value.len() <= 998;
    for value in [
        outgoing.subject.as_str(),
        outgoing.header_id.as_str(),
        outgoing.references.as_str(),
        outgoing.in_reply_to.as_deref().unwrap_or(""),
    ] {
        if !header_ok(value) {
            return Err(invalid("header"));
        }
    }
    let address = |value: &str| value.parse::<Address>().map_err(|_| invalid("address"));
    let name = one_line(&outgoing.from_name, 80);
    let mut builder = Message::builder()
        .from(Mailbox::new(
            (!name.is_empty()).then_some(name),
            address(&outgoing.from_address)?,
        ))
        .to(Mailbox::new(None, address(&outgoing.to)?))
        .subject(outgoing.subject.clone())
        .message_id(Some(format!("<{}>", outgoing.header_id)));
    if let Some(parent) = &outgoing.in_reply_to {
        builder = builder.in_reply_to(format!("<{parent}>"));
    }
    if !outgoing.references.is_empty() {
        let references: Vec<String> = outgoing
            .references
            .split_whitespace()
            .map(|id| format!("<{id}>"))
            .collect();
        builder = builder.references(references.join(" "));
    }
    builder
        .header(ContentType::TEXT_PLAIN)
        .body(outgoing.body.clone())
        .map_err(|_| invalid("body"))
}

pub fn register_job(
    worker: Worker,
    database: Database,
    app_key: Option<[u8; 32]>,
    development: bool,
) -> Result<Worker, JobKindRegistrationError> {
    let sender = SupportSender::new(database, app_key, development);
    worker.with_handler(job_kind(), move |context| {
        let sender = sender.clone();
        async move {
            let message_id = context
                .job
                .payload
                .get("message_id")
                .and_then(Value::as_str)
                .and_then(|id| id.parse::<Id>().ok())
                .ok_or_else(|| JobError::Permanent("support.send needs a message_id".to_owned()))?;
            let last = context.claim.attempt >= context.job.max_attempts;
            match sender.deliver(message_id, last).await {
                Ok(()) => Ok(()),
                Err(failure) if failure.retry && !last => Err(JobError::Retryable(failure.text)),
                Err(failure) => Err(JobError::Permanent(failure.text)),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outgoing() -> Outgoing {
        Outgoing {
            message_id: Id::new_v7(),
            workspace_id: Id::new_v7(),
            mailbox_id: Id::new_v7(),
            from_address: "support@example.com".to_owned(),
            from_name: "Acme Support".to_owned(),
            to: "ada@example.org".to_owned(),
            subject: "Re: Login [#SUP-1]".to_owned(),
            body: "Hello Ada".to_owned(),
            header_id: "orbit.x.y.z@example.com".to_owned(),
            in_reply_to: Some("abc@example.org".to_owned()),
            references: "zero@example.org abc@example.org".to_owned(),
            smtp_host: None,
            smtp_port: None,
            smtp_security: None,
            smtp_username: None,
            smtp_password: None,
        }
    }

    #[test]
    fn a_reply_carries_threading_headers() {
        let text = String::from_utf8(build_message(&outgoing()).unwrap().formatted()).unwrap();
        assert!(
            text.contains("Message-ID: <orbit.x.y.z@example.com>"),
            "{text}"
        );
        assert!(text.contains("In-Reply-To: <abc@example.org>"), "{text}");
        assert!(
            text.contains("References: <zero@example.org> <abc@example.org>"),
            "{text}"
        );
        assert!(text.contains("To: ada@example.org"), "{text}");
        assert!(text.contains("Hello Ada"), "{text}");
    }

    #[test]
    fn smtp_errors_explain_microsoft_answers_and_never_show_the_password() {
        let text = explain(
            "permanent error (535): 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is disabled for the Mailbox. p@ss w0rd",
            Some("p@ss w0rd"),
        );
        assert!(text.contains("Authenticated SMTP"), "{text}");
        assert!(!text.contains("p@ss w0rd"), "{text}");
        assert!(
            explain(
                "permanent error (550): 5.7.30 Basic authentication is not supported",
                None
            )
            .contains("5.7.30")
        );
        let text = explain(
            "permanent error (535): 5.7.3 Authentication unsuccessful cEBzcyB3MHJk",
            Some("p@ss w0rd"),
        );
        assert!(text.contains("user name or password"), "{text}");
        assert!(!text.contains("cEBzcyB3MHJk"), "{text}");
    }

    #[test]
    fn line_breaks_cannot_add_headers() {
        let mut bad = outgoing();
        bad.subject = "Hi\r\nBcc: victim@example.com".to_owned();
        assert!(build_message(&bad).is_err());
        let mut bad = outgoing();
        bad.to = "ada@example.org\r\nBcc: victim@example.com".to_owned();
        assert!(build_message(&bad).is_err());
        let mut bad = outgoing();
        bad.to = "ada@example.org, eve@example.com".to_owned();
        assert!(build_message(&bad).is_err());
    }
}
