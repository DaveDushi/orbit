//! How a customer's reply finds its ticket. Replies that Orbit sends carry a Message-ID with the
//! task id and an HMAC of it under the app key (`orbit.<task>.<random>.<tag>@<domain>`); a reply to
//! one names it in `In-Reply-To`/`References`. A made-up id with another task's id has no valid
//! tag, so it cannot put mail into another ticket. The second way is the `[#KEY-123]` tag in the
//! subject, which the inbound path accepts only from the ticket's own customer.

use hmac::{Hmac, Mac};
use orbit_platform::Id;
use sha2::Sha256;
use subtle::ConstantTimeEq;

use super::inbound::hex;

const PREFIX: &str = "orbit.";
/// Bytes of the HMAC kept in the id: 128 bits.
const TAG_BYTES: usize = 16;

fn tag(key: &[u8; 32], task_id: Id) -> [u8; TAG_BYTES] {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(b"orbit support thread v1\0");
    mac.update(task_id.to_string().as_bytes());
    let digest = mac.finalize().into_bytes();
    let mut tag = [0; TAG_BYTES];
    tag.copy_from_slice(&digest[..TAG_BYTES]);
    tag
}

/// A new Message-ID (without angle brackets) for a reply on the ticket `task_id`, at the domain
/// of the mailbox's address.
#[must_use]
pub fn message_id(key: &[u8; 32], task_id: Id, domain: &str) -> String {
    let random = Id::new_v7().to_string().replace('-', "");
    format!(
        "{PREFIX}{task_id}.{random}.{}@{domain}",
        hex(&tag(key, task_id))
    )
}

/// The task of one of Orbit's own Message-IDs, when its tag is valid.
#[must_use]
pub fn task_of(key: &[u8; 32], message_id: &str) -> Option<Id> {
    let local = message_id.split('@').next()?.strip_prefix(PREFIX)?;
    let mut parts = local.split('.');
    let task: Id = parts.next()?.parse().ok()?;
    let _random = parts.next()?;
    let supplied = parts.next()?;
    if parts.next().is_some() || supplied.len() != TAG_BYTES * 2 {
        return None;
    }
    let expected = hex(&tag(key, task));
    bool::from(expected.as_bytes().ct_eq(supplied.as_bytes())).then_some(task)
}

/// The project key and number of the first `[#KEY-123]` tag in a subject.
#[must_use]
pub fn subject_tag(subject: &str) -> Option<(String, i64)> {
    let mut rest = subject;
    while let Some(start) = rest.find("[#") {
        let after = &rest[start + 2..];
        let end = after.find(']')?;
        if let Some((key, number)) = after[..end].rsplit_once('-')
            && !key.is_empty()
            && key.len() <= 20
            && key.chars().all(|c| c.is_ascii_alphanumeric())
            && let Ok(number) = number.parse::<i64>()
            && number > 0
        {
            return Some((key.to_ascii_uppercase(), number));
        }
        rest = &after[end..];
    }
    None
}

/// The subject of a reply: `Re: ` once, and the ticket's tag once.
#[must_use]
pub fn reply_subject(subject: &str, identifier: &str) -> String {
    let tag = format!("[#{identifier}]");
    let mut base = subject.trim().to_owned();
    while base
        .get(..3)
        .is_some_and(|start| start.eq_ignore_ascii_case("re:"))
    {
        base = base[3..].trim_start().to_owned();
    }
    if base.is_empty() {
        base = "Your request".to_owned();
    }
    if base.contains(&tag) {
        format!("Re: {base}")
    } else {
        format!("Re: {base} {tag}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_message_ids_name_their_task() {
        let task = Id::new_v7();
        let id = message_id(&[3; 32], task, "support.example.com");
        assert!(id.ends_with("@support.example.com"), "{id}");
        assert_eq!(task_of(&[3; 32], &id), Some(task));
    }

    #[test]
    fn a_forged_or_foreign_id_names_no_task() {
        let task = Id::new_v7();
        let other = Id::new_v7();
        let id = message_id(&[3; 32], task, "x.test");
        // another server's key
        assert_eq!(task_of(&[4; 32], &id), None);
        // the tag of one task with the id of another
        let forged = id.replacen(&task.to_string(), &other.to_string(), 1);
        assert_eq!(task_of(&[3; 32], &forged), None);
        // a shortened tag, extra parts, and ids that are not ours
        let short = format!("{}@x.test", &id[..id.find('@').unwrap() - 2]);
        assert_eq!(task_of(&[3; 32], &short), None);
        assert_eq!(task_of(&[3; 32], &id.replace('@', ".x@")), None);
        assert_eq!(task_of(&[3; 32], "CAH123@mail.gmail.com"), None);
        assert_eq!(
            task_of(&[3; 32], &format!("orbit.{task}.x.00@x.test")),
            None
        );
    }

    #[test]
    fn subject_tags_are_found() {
        assert_eq!(
            subject_tag("Re: Login [#SUP-12]"),
            Some(("SUP".to_owned(), 12))
        );
        assert_eq!(
            subject_tag("[#x] then [#ab2-7]"),
            Some(("AB2".to_owned(), 7))
        );
        assert_eq!(subject_tag("[#SUP-0]"), None);
        assert_eq!(subject_tag("[#SUP-12"), None);
        assert_eq!(subject_tag("[#S P-1]"), None);
        assert_eq!(subject_tag("SUP-12"), None);
    }

    #[test]
    fn reply_subjects_have_one_re_and_one_tag() {
        assert_eq!(reply_subject("Login", "SUP-1"), "Re: Login [#SUP-1]");
        assert_eq!(
            reply_subject("RE: re: Login [#SUP-1]", "SUP-1"),
            "Re: Login [#SUP-1]"
        );
        assert_eq!(reply_subject("  ", "SUP-1"), "Re: Your request [#SUP-1]");
        assert_eq!(reply_subject("Ré", "SUP-1"), "Re: Ré [#SUP-1]");
    }
}
