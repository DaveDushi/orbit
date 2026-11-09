//! The update check (design decision D7, option 1): the app reads
//! `https://<server>/downloads/desktop/latest.json`, a static file next to the server (behind the
//! gateway like the rest), and shows "Update available" when that version is newer than its own.
//! It downloads nothing itself: the link opens in the browser.
//!
//! ```json
//! { "version": "0.2.0", "notes": "…", "macos": "https://…/Orbit-0.2.0.dmg", "windows": "https://…/Orbit-0.2.0-windows.zip" }
//! ```

use std::cmp::Ordering;

use serde::Deserialize;

/// Where the file is, on the app's server.
pub const LATEST_PATH: &str = "/downloads/desktop/latest.json";

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Latest {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub macos: Option<String>,
    #[serde(default)]
    pub windows: Option<String>,
    #[serde(default)]
    pub linux: Option<String>,
}

impl Latest {
    /// The download for this OS, if the file has one.
    pub fn download(&self) -> Option<&str> {
        if cfg!(target_os = "macos") {
            self.macos.as_deref()
        } else if cfg!(windows) {
            self.windows.as_deref()
        } else {
            self.linux.as_deref()
        }
    }
}

/// `Some` when `latest` is newer than `current` and can be read. A version the app cannot read
/// is never "newer": a broken file must not nag.
pub fn newer<'a>(current: &str, latest: &'a Latest) -> Option<&'a Latest> {
    (compare(&latest.version, current)? == Ordering::Greater).then_some(latest)
}

/// Compares `major.minor.patch[-pre]` versions (a leading `v` is fine). A pre-release is older
/// than its release; two pre-releases of one release compare by their text.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (a_core, a_pre) = parse(a)?;
    let (b_core, b_pre) = parse(b)?;
    Some(a_core.cmp(&b_core).then_with(|| match (a_pre, b_pre) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => a.cmp(b),
    }))
}

fn parse(version: &str) -> Option<([u64; 3], Option<&str>)> {
    let version = version.trim();
    let version = version.strip_prefix('v').unwrap_or(version);
    let version = version.split('+').next()?;
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) if !pre.is_empty() => (core, Some(pre)),
        Some(_) => return None,
        None => (version, None),
    };
    let mut parts = core.split('.');
    let mut out = [0u64; 3];
    for slot in &mut out {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    parts.next().is_none().then_some((out, pre))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert_eq!(compare("0.10.0", "0.9.9"), Some(Ordering::Greater));
        assert_eq!(compare("v1.2.3", "1.2.3"), Some(Ordering::Equal));
        assert_eq!(compare("1.2.3+build.5", "1.2.3"), Some(Ordering::Equal));
        assert_eq!(compare("1.0.0-rc.1", "1.0.0"), Some(Ordering::Less));
        assert_eq!(compare("1.0.0", "1.0.0-rc.1"), Some(Ordering::Greater));
        assert_eq!(compare("1.0.0-rc.2", "1.0.0-rc.1"), Some(Ordering::Greater));
        for broken in ["", "1.2", "1.2.3.4", "1.x.3", "1..3", "1.2.3-", "latest"] {
            assert_eq!(compare(broken, "1.0.0"), None, "{broken:?}");
        }
    }

    #[test]
    fn only_a_readable_newer_version_is_offered() {
        let latest: Latest = serde_json::from_str(r#"{"version":"0.2.0","macos":"https://x/Orbit.dmg","future":true}"#).unwrap();
        assert!(newer("0.1.0", &latest).is_some());
        assert!(newer("0.2.0", &latest).is_none(), "same version");
        assert!(newer("0.3.0", &latest).is_none(), "older file (a rollback) does not nag");
        assert!(newer("0.1.0", &Latest { version: "soon".into(), ..Latest::default() }).is_none());
    }
}
