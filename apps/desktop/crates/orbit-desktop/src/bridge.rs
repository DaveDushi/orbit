//! The messages of the bridge script (`assets/bridge.js`) and the unread badge they drive.
//! A page can send anything through `window.ipc`: the app reads only these shapes, clipped to
//! sane lengths, and only from the server's own pages (the shell checks the sender).

use serde::Deserialize;

const BRIDGE_JS: &str = include_str!("../assets/bridge.js");

/// The bridge script for `origin` (`https://host[:port]`).
pub fn script(origin: &str) -> String {
    BRIDGE_JS.replace("__ORBIT_ORIGIN__", &serde_json::to_string(origin).expect("a string serializes"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Show an OS notification; `id` goes back to the page on a click.
    Notify { id: u64, title: String, body: String, tag: String },
    Badge(Badge),
    /// A link to another site: open it outside (after `route::route`).
    Open(String),
    /// The text of `/downloads/desktop/latest.json`.
    Latest(String),
    /// The server picked on the app's own start page.
    Server(String),
}

/// What the web app put on its badge (`navigator.setAppBadge`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    Count(u32),
    /// `setAppBadge()` without a number: "something is unread".
    Flag,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Wire {
    Notify {
        id: u64,
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        tag: String,
    },
    Badge {
        count: Option<f64>,
    },
    Open {
        url: String,
    },
    Latest {
        json: String,
    },
    Server {
        url: String,
    },
}

const MAX_TITLE: usize = 200;
const MAX_BODY: usize = 1_000;
const MAX_URL: usize = 4_096;
const MAX_LATEST: usize = 16_384;

pub fn parse(text: &str) -> Option<Message> {
    let wire: Wire = serde_json::from_str(text).ok()?;
    Some(match wire {
        Wire::Notify { id, title, body, tag } => {
            let title = clip(title.trim(), MAX_TITLE);
            if title.is_empty() {
                return None;
            }
            Message::Notify { id, title, body: clip(body.trim(), MAX_BODY), tag: clip(&tag, MAX_TITLE) }
        }
        // The Badging API: a negative or broken number is an error there, zero clears.
        Wire::Badge { count: None } => Message::Badge(Badge::Flag),
        Wire::Badge { count: Some(n) } if n.is_finite() && n >= 0.0 => Message::Badge(Badge::Count(n.min(f64::from(u32::MAX)) as u32)),
        Wire::Badge { .. } => return None,
        Wire::Open { url } if url.len() <= MAX_URL => Message::Open(url),
        Wire::Latest { json } if json.len() <= MAX_LATEST => Message::Latest(json),
        Wire::Server { url } if url.len() <= MAX_URL => Message::Server(url),
        Wire::Open { .. } | Wire::Latest { .. } | Wire::Server { .. } => return None,
    })
}

fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

/// The badge on the taskbar / dock icon and the tray. The web app's own count wins once it sent
/// one (a web app with `setAppBadge`); before that, the notifications that came while the window
/// was not in front count, until the person brings it to the front.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Unread {
    web: Option<Badge>,
    unseen: u32,
}

impl Unread {
    pub fn set_web(&mut self, badge: Badge) {
        self.web = Some(badge);
    }

    pub fn notified(&mut self, window_in_front: bool) {
        if !window_in_front {
            self.unseen = self.unseen.saturating_add(1);
        }
    }

    pub fn seen(&mut self) {
        self.unseen = 0;
    }

    /// A new page (another server or a sign-out): its count is not known yet.
    pub fn reset(&mut self) {
        *self = Unread::default();
    }

    pub fn badge(&self) -> Badge {
        self.web.unwrap_or(Badge::Count(self.unseen))
    }

    /// The badge text: `None` for no badge, `"•"` for a flag, `"99+"` above 99.
    pub fn label(&self) -> Option<String> {
        match self.badge() {
            Badge::Count(0) => None,
            Badge::Count(n) if n > 99 => Some("99+".into()),
            Badge::Count(n) => Some(n.to_string()),
            Badge::Flag => Some("•".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notifications_are_read_and_clipped() {
        assert_eq!(
            parse(r#"{"kind":"notify","id":3,"title":" Ada in #general ","body":"Lunch?","tag":"c1","url":"/chat/c1"}"#),
            Some(Message::Notify { id: 3, title: "Ada in #general".into(), body: "Lunch?".into(), tag: "c1".into() })
        );
        let long = format!(r#"{{"kind":"notify","id":1,"title":"{}","body":"{}"}}"#, "é".repeat(500), "x".repeat(5000));
        let Some(Message::Notify { title, body, tag, .. }) = parse(&long) else { panic!("a long notice is still a notice") };
        assert_eq!(title.chars().count(), MAX_TITLE + 1, "clipped on a character, with an ellipsis");
        assert_eq!(body.chars().count(), MAX_BODY + 1);
        assert_eq!(tag, "");
        assert_eq!(parse(r#"{"kind":"notify","id":1,"title":"   "}"#), None, "nothing to show");
        assert_eq!(parse(r#"{"kind":"notify","title":"no id"}"#), None);
    }

    #[test]
    fn badges_follow_the_badging_api() {
        assert_eq!(parse(r#"{"kind":"badge","count":4}"#), Some(Message::Badge(Badge::Count(4))));
        assert_eq!(parse(r#"{"kind":"badge","count":0}"#), Some(Message::Badge(Badge::Count(0))));
        assert_eq!(parse(r#"{"kind":"badge","count":null}"#), Some(Message::Badge(Badge::Flag)));
        assert_eq!(parse(r#"{"kind":"badge","count":2.9}"#), Some(Message::Badge(Badge::Count(2))));
        assert_eq!(parse(r#"{"kind":"badge","count":-1}"#), None);
        assert_eq!(parse(r#"{"kind":"badge","count":"many"}"#), None);
    }

    #[test]
    fn other_messages_and_junk() {
        assert_eq!(parse(r#"{"kind":"open","url":"https://github.com"}"#), Some(Message::Open("https://github.com".into())));
        assert_eq!(parse(r#"{"kind":"latest","json":"{\"version\":\"0.2.0\"}"}"#), Some(Message::Latest(r#"{"version":"0.2.0"}"#.into())));
        assert_eq!(parse(r#"{"kind":"server","url":"orbit.example.com"}"#), Some(Message::Server("orbit.example.com".into())));
        assert_eq!(parse(&format!(r#"{{"kind":"open","url":"https://x/{}"}}"#, "a".repeat(MAX_URL))), None);
        for junk in ["", "null", "[]", r#"{"kind":"eval","code":"rm -rf"}"#, r#"{"title":"no kind"}"#, "{broken"] {
            assert_eq!(parse(junk), None, "{junk}");
        }
    }

    #[test]
    fn the_badge_counts_unseen_notices_until_the_web_app_sends_its_own() {
        let mut unread = Unread::default();
        assert_eq!(unread.label(), None);
        unread.notified(true);
        assert_eq!(unread.label(), None, "the person is looking at the window");
        unread.notified(false);
        unread.notified(false);
        assert_eq!(unread.label().as_deref(), Some("2"));
        unread.seen();
        assert_eq!(unread.label(), None);

        unread.notified(false);
        unread.set_web(Badge::Count(7));
        assert_eq!(unread.label().as_deref(), Some("7"), "the web app's count wins");
        unread.seen();
        assert_eq!(unread.label().as_deref(), Some("7"), "only reading in the app lowers it");
        unread.set_web(Badge::Count(120));
        assert_eq!(unread.label().as_deref(), Some("99+"));
        unread.set_web(Badge::Flag);
        assert_eq!(unread.label().as_deref(), Some("•"));
        unread.set_web(Badge::Count(0));
        assert_eq!(unread.label(), None);

        unread.reset();
        unread.notified(false);
        assert_eq!(unread.label().as_deref(), Some("1"), "after a reset the notices count again");
    }

    #[test]
    fn the_script_names_the_server() {
        let js = script("https://orbit.example.com");
        assert!(js.contains(r#"const ORIGIN = "https://orbit.example.com""#));
        assert!(!js.contains("__ORBIT_ORIGIN__"));
    }
}
