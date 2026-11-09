//! The app's settings, a plain JSON file (`<config dir>/orbit-desktop/settings.json`): the server,
//! "Start with Windows" (off until the person turns it on) and the main window's place and size.
//! No secrets: the web view keeps the gateway and Orbit cookies in its own profile.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The server's origin, as `ServerUrl::origin` wrote it.
    pub server: Option<String>,
    pub start_with_os: bool,
    pub window: Option<WindowState>,
}

/// The main window in physical pixels: the outer position and the inner size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub maximized: bool,
}

/// A monitor's area in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub const MIN_WIDTH: u32 = 480;
pub const MIN_HEIGHT: u32 = 360;
/// How much of the title bar must be on a monitor for the person to grab it.
const GRAB: i64 = 48;

impl WindowState {
    /// The saved window, if it still fits: at least the minimum size, no larger than the largest
    /// monitor, and its top edge reachable on one of `monitors` (a monitor that was unplugged must
    /// not leave the window off screen). `None`: let the OS place a default window.
    pub fn fit(self, monitors: &[Area]) -> Option<WindowState> {
        let widest = monitors.iter().map(|m| m.width).max()?;
        let tallest = monitors.iter().map(|m| m.height).max()?;
        let state = WindowState {
            width: self.width.clamp(MIN_WIDTH, widest.max(MIN_WIDTH)),
            height: self.height.clamp(MIN_HEIGHT, tallest.max(MIN_HEIGHT)),
            ..self
        };
        let (left, top, right) = (i64::from(state.x), i64::from(state.y), i64::from(state.x) + i64::from(state.width));
        let reachable = monitors.iter().any(|m| {
            let (m_left, m_top) = (i64::from(m.x), i64::from(m.y));
            let (m_right, m_bottom) = (m_left + i64::from(m.width), m_top + i64::from(m.height));
            top >= m_top && top + GRAB <= m_bottom && right.min(m_right) - left.max(m_left) >= GRAB
        });
        reachable.then_some(state)
    }
}

impl Settings {
    /// The settings in `path`; a missing or unreadable file gives the defaults (a broken file
    /// must not stop the app).
    pub fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write then rename: a crash mid-write keeps the old file.
        let next = path.with_extension("json.next");
        std::fs::write(&next, serde_json::to_vec_pretty(self).expect("plain data serializes"))?;
        std::fs::rename(next, path)
    }

    pub fn load() -> Settings {
        path().map(|p| Settings::load_from(&p)).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Some(path) = path()
            && let Err(e) = self.save_to(&path)
        {
            eprintln!("orbit-desktop: cannot save {}: {e}", path.display());
        }
    }
}

fn path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("orbit-desktop").join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: Area = Area { x: 0, y: 0, width: 1920, height: 1080 };
    const RIGHT: Area = Area { x: 1920, y: 0, width: 2560, height: 1440 };

    fn window(x: i32, y: i32, width: u32, height: u32) -> WindowState {
        WindowState { x, y, width, height, maximized: false }
    }

    #[test]
    fn a_window_on_screen_comes_back_as_it_was() {
        let saved = WindowState { maximized: true, ..window(100, 80, 1200, 800) };
        assert_eq!(saved.fit(&[LAPTOP]), Some(saved));
        assert_eq!(window(2000, 100, 1400, 900).fit(&[LAPTOP, RIGHT]), Some(window(2000, 100, 1400, 900)));
    }

    #[test]
    fn a_window_on_an_unplugged_monitor_or_off_screen_is_placed_again() {
        assert_eq!(window(2000, 100, 1400, 900).fit(&[LAPTOP]), None, "the right monitor is gone");
        assert_eq!(window(-5000, 100, 800, 600).fit(&[LAPTOP]), None);
        assert_eq!(window(100, -300, 800, 600).fit(&[LAPTOP]), None, "title bar above the screen");
        assert_eq!(window(100, 1060, 800, 600).fit(&[LAPTOP]), None, "title bar below the screen");
        assert_eq!(window(1900, 100, 800, 600).fit(&[LAPTOP]), None, "only 20 px left to grab");
        assert_eq!(window(100, 100, 800, 600).fit(&[]), None, "no monitor known");
    }

    #[test]
    fn sizes_are_kept_between_the_minimum_and_the_largest_monitor() {
        assert_eq!(window(0, 0, 100, 50).fit(&[LAPTOP]), Some(window(0, 0, MIN_WIDTH, MIN_HEIGHT)));
        assert_eq!(window(0, 0, 9000, 9000).fit(&[LAPTOP]), Some(window(0, 0, 1920, 1080)));
    }

    #[test]
    fn start_with_the_os_is_off_until_turned_on_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("orbit-desktop").join("settings.json");
        let fresh = Settings::load_from(&path);
        assert!(!fresh.start_with_os, "off by default");
        assert_eq!(fresh, Settings::default());

        let settings = Settings { server: Some("https://orbit.example.com".into()), start_with_os: true, window: Some(window(10, 20, 900, 700)) };
        settings.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), settings);

        // An older file without the newer keys, and a broken file, both read as defaults where they lack a value.
        std::fs::write(&path, r#"{"server":"https://orbit.example.com"}"#).unwrap();
        assert_eq!(Settings::load_from(&path), Settings { server: Some("https://orbit.example.com".into()), ..Settings::default() });
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(Settings::load_from(&path), Settings::default());
    }
}
