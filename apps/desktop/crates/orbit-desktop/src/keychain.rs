//! Secrets in the OS keychain (macOS Keychain, Windows Credential Manager, Secret Service on
//! Linux): the gateway cookies and the Orbit session of each server, as JSON, one entry each.
//! Errors say which entry failed, never what it holds.

use orbit_client::{GatewayCookies, ServerUrl, Session};
use serde::Serialize;
use serde::de::DeserializeOwned;

const SERVICE: &str = "dev.orbit.desktop";

fn entry(kind: &str, server: &ServerUrl) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("{kind}:{}", server.origin())).map_err(|e| format!("keychain ({kind}): {e}"))
}

fn load<T: DeserializeOwned>(kind: &str, server: &ServerUrl) -> Option<T> {
    let secret = entry(kind, server).ok()?.get_password().ok()?;
    serde_json::from_str(&secret).ok()
}

fn save<T: Serialize>(kind: &str, server: &ServerUrl, value: &T) -> Result<(), String> {
    let json = serde_json::to_string(value).map_err(|_| format!("keychain ({kind}): cannot encode"))?;
    entry(kind, server)?.set_password(&json).map_err(|e| format!("keychain ({kind}): {e}"))
}

fn forget(kind: &str, server: &ServerUrl) {
    if let Ok(entry) = entry(kind, server) {
        let _ = entry.delete_credential();
    }
}

pub fn gateway(server: &ServerUrl) -> Option<GatewayCookies> {
    load("gateway", server)
}

pub fn save_gateway(server: &ServerUrl, cookies: &GatewayCookies) -> Result<(), String> {
    save("gateway", server, cookies)
}

pub fn forget_gateway(server: &ServerUrl) {
    forget("gateway", server);
}

pub fn session(server: &ServerUrl) -> Option<Session> {
    load("session", server)
}

pub fn save_session(server: &ServerUrl, session: &Session) -> Result<(), String> {
    save("session", server, session)
}

pub fn forget_session(server: &ServerUrl) {
    forget("session", server);
}

/// The last server, in a plain file: an address is not a secret.
pub fn last_server() -> Option<ServerUrl> {
    let path = dirs::config_dir()?.join("orbit-desktop").join("server");
    ServerUrl::parse(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn remember_server(server: &ServerUrl) {
    if let Some(dir) = dirs::config_dir().map(|d| d.join("orbit-desktop")) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("server"), server.origin());
    }
}
