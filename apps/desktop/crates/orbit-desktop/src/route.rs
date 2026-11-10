//! Where links and downloads go.
//!
//! The main window shows the server's own pages and the gateway's sign-in only. A link to
//! another site (a click in a message, a `target="_blank"` link, `window.open`) opens in the
//! default browser; `mailto:` and `tel:` go to their OS apps. A blank popup is the exception: the
//! web app opens one for an embedded app's sign-in and then loads the app's login in it, and that
//! sign-in must happen in this app's web view, where the embedded app reads its cookies. Top-level navigations between
//! pages are left alone (the gateway sign-in can pass through exe.dev and an identity provider,
//! and embeds load in frames); the bridge script sends link clicks on the server's pages here.

use std::path::{Path, PathBuf};

use orbit_client::ServerUrl;

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Load it in the main window.
    InApp,
    /// Hand it to the OS: the default browser, mail or phone app.
    External,
    /// Neither (`javascript:`, `file:`, unknown schemes).
    Deny,
    /// A popup window of the web view itself, which the page then fills (`window.open('about:blank')`).
    Popup,
}

/// A new window that a page asked for (`window.open`, `target="_blank"`).
pub fn new_window(server: &ServerUrl, url: &str) -> Route {
    // an embedded app's sign-in (apps/web `features/apps/signInPopup.ts`); WebKit may give a blank page no URL
    if url.is_empty() || url == "about:blank" { Route::Popup } else { route(server, url) }
}

/// A link the person opened, from a page in the main window.
pub fn route(server: &ServerUrl, url: &str) -> Route {
    if same_origin(server, url) {
        return Route::InApp;
    }
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase()).unwrap_or_default();
    match scheme.as_str() {
        "https" | "http" | "mailto" | "tel" => Route::External,
        _ => Route::Deny,
    }
}

/// `url` is on the server's origin (not a look-alike host such as `orbit.example.com.evil.test`).
pub fn same_origin(server: &ServerUrl, url: &str) -> bool {
    url.strip_prefix(server.origin()).is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// A page of the server that is not the gateway's sign-in: the bridge works there only.
pub fn orbit_page(server: &ServerUrl, url: &str) -> bool {
    same_origin(server, url) && !url[server.origin().len()..].starts_with("/__exe.dev")
}

/// Where a download goes: `dir` and the file's own name, with " (2)", " (3)" … before the
/// extension when that name is taken. A name with a path in it keeps only its last part.
pub fn download_path(dir: &Path, suggested: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let name = suggested.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let name = match name {
        "" | "." | ".." => "download",
        name => name,
    };
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    let mut path = dir.join(name);
    let mut n = 2;
    while taken(&path) {
        path = dir.join(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_servers_pages_stay_and_other_links_open_outside() {
        let server = ServerUrl::parse("https://orbit.example.com").unwrap();
        assert_eq!(route(&server, "https://orbit.example.com/chat/c1?thread=m1"), Route::InApp);
        assert_eq!(route(&server, "https://orbit.example.com"), Route::InApp);
        assert_eq!(route(&server, "https://github.com/wellrox/orbit/pull/1"), Route::External);
        assert_eq!(route(&server, "https://orbit.example.com.evil.test/login"), Route::External, "look-alike host");
        assert_eq!(route(&server, "https://orbit.example.com:8443/"), Route::External, "another port is another origin");
        assert_eq!(route(&server, "http://orbit.example.com/"), Route::External, "another scheme is another origin");
        assert_eq!(route(&server, "mailto:ada@example.com"), Route::External);
        assert_eq!(route(&server, "TEL:+4912345"), Route::External);
        assert_eq!(route(&server, "javascript:alert(1)"), Route::Deny);
        assert_eq!(route(&server, "file:///etc/passwd"), Route::Deny);
        assert_eq!(route(&server, "ms-settings:privacy"), Route::Deny);
    }

    #[test]
    fn a_blank_popup_opens_in_the_app_and_other_new_windows_as_links() {
        let server = ServerUrl::parse("https://orbit.example.com").unwrap();
        // the sign-in popup of an embedded app: denied, `window.open` returned null and the web app
        // said "Your browser blocked the sign-in window"
        assert_eq!(new_window(&server, "about:blank"), Route::Popup);
        assert_eq!(new_window(&server, ""), Route::Popup);
        assert_eq!(new_window(&server, "about:blank#x"), Route::Deny);
        assert_eq!(new_window(&server, "https://creators.example.com/login"), Route::External);
        assert_eq!(new_window(&server, "https://orbit.example.com/chat"), Route::InApp);
        assert_eq!(new_window(&server, "javascript:alert(1)"), Route::Deny);
    }

    #[test]
    fn the_bridge_works_on_orbit_pages_only() {
        let server = ServerUrl::parse("http://127.0.0.1:8080").unwrap();
        assert!(orbit_page(&server, "http://127.0.0.1:8080/chat"));
        assert!(!orbit_page(&server, "http://127.0.0.1:8080/__exe.dev/login?redirect=%2F"));
        assert!(!orbit_page(&server, "http://127.0.0.1:80800/chat"));
        assert!(!orbit_page(&server, "about:blank"));
    }

    #[test]
    fn downloads_keep_their_name_and_never_overwrite() {
        let dir = Path::new("/home/ada/Downloads");
        let taken = |names: &'static [&'static str]| move |p: &Path| names.iter().any(|n| p == dir.join(n));
        assert_eq!(download_path(dir, "report.pdf", taken(&[])), dir.join("report.pdf"));
        assert_eq!(download_path(dir, "report.pdf", taken(&["report.pdf", "report (2).pdf"])), dir.join("report (3).pdf"));
        assert_eq!(download_path(dir, "archive.tar.gz", taken(&["archive.tar.gz"])), dir.join("archive.tar (2).gz"));
        assert_eq!(download_path(dir, "README", taken(&["README"])), dir.join("README (2)"));
        assert_eq!(download_path(dir, ".env", taken(&[])), dir.join(".env"));
        assert_eq!(download_path(dir, "../../.bashrc", taken(&[])), dir.join(".bashrc"), "no way out of the folder");
        assert_eq!(download_path(dir, r"C:\Windows\evil.dll", taken(&[])), dir.join("evil.dll"));
        assert_eq!(download_path(dir, "", taken(&[])), dir.join("download"));
        assert_eq!(download_path(dir, "..", taken(&[])), dir.join("download"));
    }
}
