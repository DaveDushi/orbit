//! A credential the app holds: an Orbit session token or the gateway's cookie header. Its `Debug`
//! and `Display` print `[REDACTED]`, so it never reaches a log, a panic message or an error by
//! accident. Only [`Secret::expose`] gives the value, at the one place a header is built.

use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_and_display_never_show_the_value() {
        let secret = Secret::new("tok-123");
        assert_eq!(format!("{secret:?} {secret} {:?}", Some(&secret)), "[REDACTED] [REDACTED] Some([REDACTED])");
        assert_eq!(secret.expose(), "tok-123");
    }
}
