//! Windows: the AppUserModelID that names the app in toasts and groups its taskbar button.
//!
//! The installer's Start-menu shortcut carries this ID (`packaging/windows/orbit.iss`), and the app
//! sets it on its process, so Windows shows "Orbit" with its icon as the toast's sender. A toast
//! with an ID that no shortcut carries does not show at all, so without the shortcut (the zip) the
//! toasts keep notify-rust's default sender, "Windows PowerShell".

use std::path::PathBuf;

pub const APP_ID: &str = "Wellrox.Orbit";

/// Sets the ID on this process (before the first window, so the taskbar button gets it).
pub fn set_for_process() {
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    let wide: Vec<u16> = APP_ID.encode_utf16().chain(Some(0)).collect();
    // SAFETY: a NUL-terminated UTF-16 string that lives for the call.
    unsafe {
        SetCurrentProcessExplicitAppUserModelID(wide.as_ptr());
    }
}

/// The ID for toasts: ours when the installer's Start-menu shortcut is there, else `None`.
pub fn for_toasts() -> Option<&'static str> {
    shortcut().is_some_and(|path| path.exists()).then_some(APP_ID)
}

/// `{userprograms}\Orbit.lnk` of the per-user install.
fn shortcut() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(r"Microsoft\Windows\Start Menu\Programs\Orbit.lnk"))
}
