//! A raw inbound email (RFC 5322 MIME, as the mail provider received it) parsed with
//! `mail-parser` into what a ticket keeps: sender, threading headers, bodies and attachments.
//! The HTML body is still raw here; `support::html` cleans it.

use mail_parser::{HeaderValue, MessageParser, MimeHeaders};
use sha2::{Digest, Sha256};

/// The largest raw email Orbit takes: 25 MB of attachments grow by a third in base64, plus bodies.
pub const MAX_RAW_BYTES: usize = 36 * 1024 * 1024;
/// All attachments of one email together; more are left out (the message says how many).
pub const MAX_ATTACHMENT_BYTES: usize = 25 * 1024 * 1024;
/// Attachments of one email; more are left out.
pub const MAX_ATTACHMENTS: usize = 20;
const MAX_TEXT_CHARS: usize = 200_000;
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
const MAX_SUBJECT_CHARS: usize = 250;
const MAX_NAME_CHARS: usize = 200;
const MAX_ADDRESSES: usize = 50;
const MAX_THREAD_IDS: usize = 50;
const MAX_MESSAGE_ID_CHARS: usize = 500;

#[derive(Clone, Debug, Default)]
pub struct InboundEmail {
    /// Lowercase.
    pub from_address: String,
    pub from_name: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    /// Without angle brackets. A message without one gets a stable id made from its bytes.
    pub message_id: String,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub text: String,
    /// The HTML body as sent, not yet cleaned.
    pub html: Option<String>,
    pub attachments: Vec<InboundAttachment>,
    /// Attachments left out by the size or count cap.
    pub skipped_attachments: usize,
    /// An auto-reply, bounce or bulk message: Orbit drops it, so two auto-responders cannot loop.
    pub automatic: bool,
    /// The provider's `Authentication-Results` say DMARC failed, or SPF failed without a DKIM pass.
    pub unverified: bool,
    /// `Reply-To`, when it names one valid address: (address, name).
    pub reply_to: Option<(String, String)>,
    /// The sender of a forwarded email: `X-Original-From`, or the `From:` line of the forwarded
    /// block in the text (Outlook "Forward", Gmail "Forwarded message").
    pub forwarded_from: Option<(String, String)>,
    /// Lowercase domains with `dmarc=pass` (`header.from`) or `dkim=pass` (`header.d`) in the
    /// topmost `Authentication-Results`, only when Cloudflare Email Routing wrote it.
    pub authenticated_domains: Vec<String>,
}

impl InboundEmail {
    /// The customer of this email. A redirect (Microsoft 365 "Redirect", a transport rule, a
    /// provider's forwarding) keeps their `From`. A forward by someone of the support domain
    /// (an Outlook "Forward" rule on the support mailbox or a colleague's mailbox) puts the
    /// mailbox or the colleague in `From`: then the customer comes from the forwarded `From:` or
    /// `Reply-To`, when that is outside the support domain and the receiving hop proved that the
    /// email comes from the support domain. Otherwise `From` stays.
    #[must_use]
    pub fn requester(&self, mailbox_address: &str) -> (String, String) {
        match self.forward(mailbox_address) {
            Some((customer, true)) => customer.clone(),
            _ => (self.from_address.clone(), self.from_name.clone()),
        }
    }

    /// `unverified`, or a forward from the support domain that names another customer without
    /// proof that it comes from the support domain: anyone can write such a `From`.
    #[must_use]
    pub fn unverified_for(&self, mailbox_address: &str) -> bool {
        self.unverified || matches!(self.forward(mailbox_address), Some((_, false)))
    }

    /// The customer that a forward from the support domain names, and whether the email is proven
    /// to come from the support domain.
    fn forward(&self, mailbox_address: &str) -> Option<(&(String, String), bool)> {
        let domain = |address: &str| {
            address
                .rsplit_once('@')
                .map(|(_, d)| d.to_ascii_lowercase())
        };
        let ours = domain(mailbox_address)?;
        let inside = |address: &str| {
            address.eq_ignore_ascii_case(mailbox_address) || domain(address).as_ref() == Some(&ours)
        };
        if !inside(&self.from_address) {
            return None;
        }
        let customer = [&self.forwarded_from, &self.reply_to]
            .into_iter()
            .flatten()
            .find(|(address, _)| !inside(address))?;
        let proven = !self.unverified && self.authenticated_domains.contains(&ours);
        Some((customer, proven))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboundAttachment {
    pub name: String,
    /// Without angle brackets: what `cid:` URLs in the HTML body name.
    pub content_id: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// No headers: not an email.
    NotEmail,
    /// No usable `From` address.
    NoSender,
}

/// Parses `raw`. Never panics; a message that is not an email or names no sender is refused.
pub fn parse(raw: &[u8]) -> Result<InboundEmail, ParseError> {
    let message = MessageParser::default()
        .parse(raw)
        .ok_or(ParseError::NotEmail)?;
    let sender = message
        .from()
        .and_then(|from| from.first())
        .ok_or(ParseError::NoSender)?;
    let from_address = sender
        .address()
        .map(|address| address.trim().to_ascii_lowercase())
        .filter(|address| valid_address(address))
        .ok_or(ParseError::NoSender)?;
    let from_name = one_line(sender.name().unwrap_or(""), MAX_NAME_CHARS);
    let addresses = |list: Option<&mail_parser::Address<'_>>| -> Vec<String> {
        list.map(|list| {
            list.iter()
                .filter_map(|addr| addr.address())
                .map(|address| address.trim().to_ascii_lowercase())
                .filter(|address| valid_address(address))
                .take(MAX_ADDRESSES)
                .collect()
        })
        .unwrap_or_default()
    };
    let headers: Vec<(String, String)> = message
        .headers_raw()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();

    let text: String = message
        .body_text(0)
        .map(|text| text.chars().take(MAX_TEXT_CHARS).collect())
        .unwrap_or_default();
    let html = message
        .html_part(0)
        .filter(|part| part.is_text_html())
        .and_then(|part| part.text_contents())
        .filter(|html| html.len() <= MAX_HTML_BYTES)
        .map(str::to_owned);

    let mut attachments = Vec::new();
    let mut skipped_attachments = 0;
    let mut total = 0_usize;
    for part in message.attachments() {
        let bytes = part.contents();
        if attachments.len() >= MAX_ATTACHMENTS
            || bytes.is_empty()
            || total + bytes.len() > MAX_ATTACHMENT_BYTES
        {
            skipped_attachments += 1;
            continue;
        }
        total += bytes.len();
        let name = part
            .attachment_name()
            .map(file_name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                if part.is_message() {
                    "message.eml".to_owned()
                } else {
                    "attachment".to_owned()
                }
            });
        attachments.push(InboundAttachment {
            name,
            content_id: part.content_id().map(bare_id).filter(|id| !id.is_empty()),
            bytes: bytes.to_vec(),
        });
    }

    let message_id = message
        .message_id()
        .map(bare_id)
        .filter(|id| !id.is_empty() && id.len() <= MAX_MESSAGE_ID_CHARS)
        .unwrap_or_else(|| {
            // A redelivery of the same bytes keeps its id, so it is not filed twice.
            let digest = Sha256::digest(raw);
            format!("generated-{}@orbit.invalid", hex(&digest[..16]))
        });
    let automatic = is_automatic(&headers, &from_address, &message);
    let forwarded_from = headers
        .iter()
        .find(|(name, _)| name == "x-original-from")
        .and_then(|(_, value)| sender_in(value))
        .or_else(|| forwarded_sender(&text));
    Ok(InboundEmail {
        to: addresses(message.to()),
        cc: addresses(message.cc()),
        subject: one_line(message.subject().unwrap_or(""), MAX_SUBJECT_CHARS),
        message_id,
        in_reply_to: thread_ids(message.in_reply_to()),
        references: thread_ids(message.references()),
        text,
        html,
        attachments,
        skipped_attachments,
        automatic,
        unverified: unverified(&headers),
        authenticated_domains: authenticated_domains(&headers),
        reply_to: message
            .reply_to()
            .and_then(|list| list.first())
            .and_then(|addr| {
                let address = addr.address()?.trim().to_ascii_lowercase();
                valid_address(&address)
                    .then(|| (address, one_line(addr.name().unwrap_or(""), MAX_NAME_CHARS)))
            }),
        forwarded_from,
        from_address,
        from_name,
    })
}

/// Auto-replies (RFC 3834 `Auto-Submitted`, vacation headers), bulk and list mail, and bounces
/// (null return path, mailer daemons, delivery reports).
fn is_automatic(
    headers: &[(String, String)],
    from_address: &str,
    message: &mail_parser::Message<'_>,
) -> bool {
    let header = |name: &str| -> Vec<String> {
        headers
            .iter()
            .filter(|(key, _)| key == name)
            .map(|(_, value)| value.to_ascii_lowercase())
            .collect()
    };
    let auto_submitted = header("auto-submitted")
        .iter()
        .any(|value| !value.starts_with("no"));
    let bulk = header("precedence")
        .iter()
        .any(|value| matches!(value.as_str(), "bulk" | "junk" | "list"));
    let vacation = !header("x-autoreply").is_empty()
        || !header("x-autorespond").is_empty()
        || (header("x-auto-response-suppress")
            .iter()
            .any(|value| value.contains("all") || value.contains("oof"))
            && !header("x-ms-exchange-generated-message-source").is_empty());
    let null_sender = header("return-path").iter().any(|value| value == "<>");
    let local = from_address.split('@').next().unwrap_or("");
    let daemon = matches!(local, "mailer-daemon" | "postmaster");
    let report = message.root_part().content_type().is_some_and(|kind| {
        kind.ctype().eq_ignore_ascii_case("multipart")
            && kind
                .subtype()
                .is_some_and(|subtype| subtype.eq_ignore_ascii_case("report"))
    });
    auto_submitted || bulk || vacation || null_sender || daemon || report
}

/// What the receiving provider wrote in `Authentication-Results`. A sender can add such a header
/// too, but only a failing one counts, so a forged "pass" gains nothing.
fn unverified(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .filter(|(name, _)| name == "authentication-results")
        .any(|(_, value)| {
            let value = value.to_ascii_lowercase();
            let has = |result: &str| {
                value
                    .split(|c: char| c.is_whitespace() || c == ';')
                    .any(|word| word == result)
            };
            // A forward breaks SPF but keeps the DKIM signature: SPF alone does not fail it.
            let dkim_pass = has("dkim=pass");
            has("dmarc=fail") || ((has("spf=fail") || has("dkim=fail")) && !dkim_pass)
        })
}

/// The receiving hop that posts mail to Orbit (`docs/support/cloudflare-email-worker.js`).
const TRUSTED_AUTHSERV_ID: &str = "mx.cloudflare.net";

/// The domains that passed DMARC or DKIM for the receiving hop. Each hop puts its header above the
/// ones it received, so only the topmost `Authentication-Results` is Cloudflare's own: a sender's
/// forged one is below it. Another provider (no Cloudflare header on top) proves nothing.
fn authenticated_domains(headers: &[(String, String)]) -> Vec<String> {
    let Some((_, value)) = headers
        .iter()
        .find(|(name, _)| name == "authentication-results")
    else {
        return Vec::new();
    };
    let value = value.to_ascii_lowercase();
    let mut clauses = value.split(';').map(str::trim);
    if clauses.next().and_then(|id| id.split_whitespace().next()) != Some(TRUSTED_AUTHSERV_ID) {
        return Vec::new();
    }
    clauses
        .filter_map(|clause| {
            let mut words = clause.split_whitespace();
            let property = match words.next()? {
                "dmarc=pass" => "header.from=",
                "dkim=pass" => "header.d=",
                _ => return None,
            };
            words
                .find_map(|word| word.strip_prefix(property))
                .map(|domain| domain.trim_matches('"').to_owned())
                .filter(|domain| !domain.is_empty())
        })
        .collect()
}

fn thread_ids(value: &HeaderValue<'_>) -> Vec<String> {
    let ids: Vec<String> = match value {
        HeaderValue::Text(id) => vec![bare_id(id)],
        HeaderValue::TextList(list) => list.iter().map(|id| bare_id(id)).collect(),
        _ => Vec::new(),
    };
    ids.into_iter()
        .filter(|id| !id.is_empty() && id.len() <= MAX_MESSAGE_ID_CHARS)
        .take(MAX_THREAD_IDS)
        .collect()
}

/// `<id@host>` → `id@host`, without spaces or control characters.
pub(crate) fn bare_id(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect()
}

/// One email address without a display name, no line breaks, at most 320 bytes.
#[must_use]
pub fn valid_address(value: &str) -> bool {
    !value.contains(['\r', '\n']) && crate::mail::valid_address(value)
}

/// The `From:` line of the first forwarded block of a text body: a line such as
/// `-----Original Message-----`, `---------- Forwarded message ---------`, `Begin forwarded
/// message:` or Outlook's `________________________________`, then within 8 lines `From:` (also
/// `Von:`, `De:`, `Van:`) with an address.
fn forwarded_sender(text: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let separator = |line: &str| {
        let lower = line.to_ascii_lowercase();
        lower.starts_with("-----")
            || lower.starts_with("_____")
            || lower.contains("forwarded message")
            || lower.contains("original message")
            || lower.starts_with("begin forwarded message")
    };
    let start = lines.iter().position(|line| separator(line))?;
    lines[start + 1..].iter().take(8).find_map(|line| {
        let (label, value) = line.split_once(':')?;
        let label = label
            .trim()
            .trim_start_matches('*')
            .trim_end_matches('*')
            .to_ascii_lowercase();
        matches!(label.as_str(), "from" | "von" | "de" | "van")
            .then(|| sender_in(value.trim_start_matches('*')))
            .flatten()
    })
}

/// The address (and name) in `Name <a@b>`, `Name [mailto:a@b]` or a bare `a@b`.
fn sender_in(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    let (name, address) = if let Some((name, rest)) = value.split_once('<') {
        (name, rest.split('>').next()?)
    } else if let Some((name, rest)) = value.split_once("[mailto:") {
        (name, rest.split(']').next()?)
    } else {
        (
            "",
            value.split_whitespace().find(|word| word.contains('@'))?,
        )
    };
    let address = address.trim().to_ascii_lowercase();
    valid_address(&address).then(|| {
        (
            address,
            one_line(name.trim().trim_matches('"'), MAX_NAME_CHARS),
        )
    })
}

/// Text on one line: control characters become spaces, runs of spaces collapse, at most `max` chars.
pub(crate) fn one_line(value: &str, max: usize) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    cleaned
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// The last path segment of a file name, on one line, without characters that file systems refuse.
fn file_name(value: &str) -> String {
    let base = value.rsplit(['/', '\\']).next().unwrap_or(value);
    one_line(base, 200)
        .chars()
        .filter(|c| !matches!(c, '"' | '<' | '>' | ':' | '|' | '?' | '*'))
        .collect::<String>()
        .trim_matches('.')
        .to_owned()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(headers: &str, body: &str) -> Vec<u8> {
        format!("{headers}\r\n\r\n{body}").into_bytes()
    }

    #[test]
    fn a_plain_message_keeps_sender_subject_and_text() {
        let email = parse(&raw(
            "From: Ada Lovelace <Ada@Example.com>\r\nTo: support@orbit.test\r\nSubject: Login\r\n\tbroken\r\nMessage-ID: <abc@example.com>",
            "Hello,\r\nI cannot log in.\r\n",
        ))
        .unwrap();
        assert_eq!(email.from_address, "ada@example.com");
        assert_eq!(email.from_name, "Ada Lovelace");
        assert_eq!(email.to, vec!["support@orbit.test"]);
        assert_eq!(email.subject, "Login broken");
        assert_eq!(email.message_id, "abc@example.com");
        assert!(email.text.contains("I cannot log in."));
        assert!(email.html.is_none());
        assert!(!email.automatic);
        assert!(!email.unverified);
    }

    #[test]
    fn encoded_headers_are_decoded() {
        let email = parse(&raw(
            "From: =?UTF-8?B?SsO8cmdlbg==?= <j@example.com>\r\nSubject: =?UTF-8?Q?Gr=C3=BC=C3=9Fe_aus_K=C3=B6ln?=",
            "x",
        ))
        .unwrap();
        assert_eq!(email.from_name, "Jürgen");
        assert_eq!(email.subject, "Grüße aus Köln");
    }

    #[test]
    fn multipart_keeps_text_html_attachments_and_content_ids() {
        let body = "--outer\r\n\
Content-Type: multipart/related; boundary=\"rel\"\r\n\r\n\
--rel\r\n\
Content-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n\
--alt\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain body\r\n\
--alt\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>HTML <img src=\"cid:logo@x\"></p>\r\n\
--alt--\r\n\
--rel\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline; filename=\"logo.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n\
--rel--\r\n\
--outer\r\nContent-Type: application/pdf; name=\"../../invoice.pdf\"\r\nContent-Disposition: attachment; filename=\"../../invoice.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=\r\n\
--outer--\r\n";
        let email = parse(&raw(
            "From: a@example.com\r\nSubject: Files\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"outer\"",
            body,
        ))
        .unwrap();
        assert!(email.text.contains("Plain body"));
        assert!(email.html.as_deref().unwrap().contains("cid:logo@x"));
        assert_eq!(email.attachments.len(), 2);
        let logo = &email.attachments[0];
        assert_eq!(logo.content_id.as_deref(), Some("logo@x"));
        assert_eq!(logo.bytes, b"\x89PNG\r\n\x1a\n");
        assert_eq!(email.attachments[1].name, "invoice.pdf");
        assert_eq!(email.attachments[1].bytes, b"%PDF-1.4");
    }

    #[test]
    fn an_html_only_message_gets_a_text_version() {
        let email = parse(&raw(
            "From: a@example.com\r\nContent-Type: text/html; charset=utf-8",
            "<p>Only <b>HTML</b></p>",
        ))
        .unwrap();
        assert!(email.html.is_some());
        assert!(email.text.contains("Only"), "{}", email.text);
    }

    #[test]
    fn attachments_over_the_count_cap_are_left_out() {
        let mut body = String::new();
        for index in 0..(MAX_ATTACHMENTS + 3) {
            body.push_str(&format!(
                "--b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"f{index}.txt\"\r\n\r\nfile {index}\r\n"
            ));
        }
        body.push_str("--b--\r\n");
        let email = parse(&raw(
            "From: a@example.com\r\nContent-Type: multipart/mixed; boundary=\"b\"",
            &body,
        ))
        .unwrap();
        assert_eq!(email.attachments.len(), MAX_ATTACHMENTS);
        assert_eq!(email.skipped_attachments, 3);
    }

    #[test]
    fn threading_headers_are_bare_ids() {
        let email = parse(&raw(
            "From: a@example.com\r\nIn-Reply-To: <one@x>\r\nReferences: <zero@x> <one@x>",
            "x",
        ))
        .unwrap();
        assert_eq!(email.in_reply_to, vec!["one@x"]);
        assert_eq!(email.references, vec!["zero@x", "one@x"]);
    }

    #[test]
    fn a_message_without_id_gets_a_stable_one() {
        let bytes = raw("From: a@example.com\r\nSubject: x", "x");
        let first = parse(&bytes).unwrap().message_id;
        assert!(first.starts_with("generated-"));
        assert_eq!(parse(&bytes).unwrap().message_id, first);
    }

    #[test]
    fn auto_replies_bounces_and_bulk_mail_are_automatic() {
        for headers in [
            "From: a@example.com\r\nAuto-Submitted: auto-replied",
            "From: a@example.com\r\nPrecedence: bulk",
            "From: a@example.com\r\nPrecedence: list",
            "From: a@example.com\r\nX-Autoreply: yes",
            "From: MAILER-DAEMON@example.com",
            "From: a@example.com\r\nReturn-Path: <>",
            "From: a@example.com\r\nContent-Type: multipart/report; report-type=delivery-status; boundary=\"r\"",
        ] {
            assert!(parse(&raw(headers, "x")).unwrap().automatic, "{headers}");
        }
        assert!(
            !parse(&raw("From: a@example.com\r\nAuto-Submitted: no", "x"))
                .unwrap()
                .automatic
        );
    }

    #[test]
    fn failed_authentication_marks_the_sender_unverified() {
        let check = |results: &str| {
            parse(&raw(
                &format!("From: a@example.com\r\nAuthentication-Results: mx.example; {results}"),
                "x",
            ))
            .unwrap()
            .unverified
        };
        assert!(check("dmarc=fail (p=none) header.from=example.com"));
        assert!(check("spf=fail smtp.mailfrom=example.com; dkim=none"));
        assert!(!check(
            "spf=fail smtp.mailfrom=example.com; dkim=pass header.d=example.com"
        ));
        assert!(!check("spf=pass; dkim=pass; dmarc=pass"));
    }

    /// What Cloudflare Email Routing writes on top for mail that passed DMARC for acme.test.
    const PROOF: &str = "Authentication-Results: mx.cloudflare.net;\r\n\tdkim=none;\r\n\tdmarc=pass (p=QUARANTINE) header.from=acme.test;\r\n\tspf=pass smtp.mailfrom=acme.test";

    #[test]
    fn a_forward_by_the_support_domain_keeps_the_original_customer() {
        // Outlook "Forward" rule on the support mailbox: From is the mailbox itself.
        let email = parse(&raw(
            &format!("{PROOF}\r\nFrom: Acme Support <support@acme.test>\r\nSubject: FW: Export broken"),
            "\r\n________________________________\r\nFrom: Ada Lovelace <ada@customer.test>\r\nSent: Monday\r\nSubject: Export broken\r\n\r\nHelp",
        ))
        .unwrap();
        assert_eq!(
            email.requester("support@acme.test"),
            ("ada@customer.test".to_owned(), "Ada Lovelace".to_owned())
        );
        assert!(!email.unverified_for("support@acme.test"));
        // a colleague of the same domain forwards with Gmail's marker and an old mailto form
        let email = parse(&raw(
            &format!("{PROOF}\r\nFrom: jane@acme.test\r\nSubject: Fwd: x"),
            "---------- Forwarded message ---------\r\nFrom: Bob [mailto:bob@client.test]\r\n",
        ))
        .unwrap();
        assert_eq!(email.requester("support@acme.test").0, "bob@client.test");
        // only Reply-To names the customer; a DKIM signature of the support domain proves it too
        let email = parse(&raw(
            "Authentication-Results: mx.cloudflare.net; dkim=pass header.d=acme.test header.s=s1\r\nFrom: support@acme.test\r\nReply-To: Ada <ada@customer.test>",
            "x",
        ))
        .unwrap();
        assert_eq!(email.requester("support@acme.test").0, "ada@customer.test");
    }

    #[test]
    fn an_unproven_forward_from_the_support_domain_keeps_from_and_is_unverified() {
        // Anyone can write `From: x@acme.test` and name any customer in the body or Reply-To:
        // without the receiving hop's proof, the reply would go where the sender chose.
        let headers = "From: x@acme.test\r\nReply-To: eve@attacker.test";
        let body = "-----Original Message-----\r\nFrom: Eve <eve@attacker.test>\r\n";
        let not_proof = [
            String::new(),
            // a forged pass below Cloudflare's own header
            format!("Authentication-Results: mx.cloudflare.net; dmarc=none\r\n{PROOF}\r\n"),
            // another authserv-id on top
            "Authentication-Results: mx.attacker.test; dmarc=pass header.from=acme.test\r\n".to_owned(),
            // a pass for another domain
            "Authentication-Results: mx.cloudflare.net; dkim=pass header.d=attacker.test; dmarc=pass header.from=attacker.test\r\n".to_owned(),
            // Cloudflare's pass, but an earlier hop (the tenant that relayed it) saw DMARC fail
            format!("{PROOF}\r\nAuthentication-Results: spf.protection.outlook.com; dmarc=fail header.from=acme.test\r\n"),
        ];
        for prefix in not_proof {
            let email = parse(&raw(&format!("{prefix}{headers}"), body)).unwrap();
            assert_eq!(
                email.requester("support@acme.test").0,
                "x@acme.test",
                "{prefix}"
            );
            assert!(email.unverified_for("support@acme.test"), "{prefix}");
        }
        // the same email with the proof names the forwarded customer
        let email = parse(&raw(&format!("{PROOF}\r\n{headers}"), body)).unwrap();
        assert_eq!(email.requester("support@acme.test").0, "eve@attacker.test");
        assert!(!email.unverified_for("support@acme.test"));
        // an own-domain email that names no other customer is not flagged
        let email = parse(&raw("From: x@acme.test", "x")).unwrap();
        assert!(!email.unverified_for("support@acme.test"));
    }

    #[test]
    fn a_redirect_or_a_customer_reply_to_keeps_from() {
        // Redirect keeps the customer's From; a Reply-To of the support domain changes nothing
        let email = parse(&raw(
            "From: Ada <ada@customer.test>\r\nReply-To: support@acme.test\r\nResent-From: support@acme.test",
            "x",
        ))
        .unwrap();
        assert_eq!(email.requester("support@acme.test").0, "ada@customer.test");
        // a customer quoting a forwarded block keeps being the requester
        let email = parse(&raw(
            "From: ada@customer.test",
            "-----Original Message-----\r\nFrom: eve@other.test\r\n",
        ))
        .unwrap();
        assert_eq!(email.requester("support@acme.test").0, "ada@customer.test");
        // a forward with no customer anywhere stays with From
        let email = parse(&raw("From: support@acme.test", "x")).unwrap();
        assert_eq!(email.requester("support@acme.test").0, "support@acme.test");
    }

    #[test]
    fn non_emails_and_missing_senders_are_refused() {
        assert_eq!(parse(b"").unwrap_err(), ParseError::NotEmail);
        assert_eq!(
            parse(&raw("Subject: no sender", "x")).unwrap_err(),
            ParseError::NoSender
        );
        assert_eq!(
            parse(&raw("From: not an address", "x")).unwrap_err(),
            ParseError::NoSender
        );
    }
}
