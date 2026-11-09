//! The address of an Orbit server: an origin only (`https://host[:port]`). Plain `http` is for a
//! server on this machine only, so a session token never crosses the network in clear text.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServerUrl {
    origin: String,
    host: String,
    secure: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ServerUrlError {
    Empty,
    Invalid,
    /// `http` to another machine.
    Insecure,
    /// A path, query, fragment or user name: the server is an origin.
    NotAnOrigin,
}

impl fmt::Display for ServerUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ServerUrlError::Empty => "Enter the server address.",
            ServerUrlError::Invalid => "This is not a server address.",
            ServerUrlError::Insecure => "Use https:// (http:// is only for a server on this computer).",
            ServerUrlError::NotAnOrigin => "Enter only the server address, without a path.",
        })
    }
}

impl std::error::Error for ServerUrlError {}

impl ServerUrl {
    /// `orbit.example.com`, `https://orbit.example.com/` and `http://127.0.0.1:8080` are accepted.
    pub fn parse(input: &str) -> Result<Self, ServerUrlError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(ServerUrlError::Empty);
        }
        let (scheme, rest) = match input.split_once("://") {
            Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
            None => ("https".to_owned(), input),
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains(['/', '?', '#', '@']) {
            return Err(ServerUrlError::NotAnOrigin);
        }
        let authority = rest.to_ascii_lowercase();
        let (host, port) = match authority.strip_prefix('[') {
            Some(v6) => {
                let (inner, after) = v6.split_once(']').ok_or(ServerUrlError::Invalid)?;
                (format!("[{inner}]"), if after.is_empty() { None } else { Some(after.strip_prefix(':').ok_or(ServerUrlError::Invalid)?) })
            }
            None => match authority.split_once(':') {
                Some((host, port)) => (host.to_owned(), Some(port)),
                None => (authority.clone(), None),
            },
        };
        let host_ok = !host.is_empty()
            && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'));
        if !host_ok || port.is_some_and(|p| p.parse::<u16>().is_err()) {
            return Err(ServerUrlError::Invalid);
        }
        let secure = match scheme.as_str() {
            "https" => true,
            "http" if is_loopback(&host) => false,
            "http" => return Err(ServerUrlError::Insecure),
            _ => return Err(ServerUrlError::Invalid),
        };
        Ok(ServerUrl { origin: format!("{scheme}://{authority}"), host, secure })
    }

    /// `https://host[:port]`, without a trailing slash.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn is_secure(&self) -> bool {
        self.secure
    }

    /// `path` starts with `/`.
    pub fn http(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }

    /// The same address with `ws`/`wss`.
    pub fn ws(&self, path: &str) -> String {
        let rest = self.origin.split_once("://").map_or("", |(_, rest)| rest);
        format!("{}://{rest}{path}", if self.secure { "wss" } else { "ws" })
    }
}

impl fmt::Display for ServerUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.origin)
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "[::1]" || host.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_become_origins() {
        let url = ServerUrl::parse(" Orbit.Example.com/ ").unwrap();
        assert_eq!((url.origin(), url.host()), ("https://orbit.example.com", "orbit.example.com"));
        assert_eq!(url.ws("/api/v1/x"), "wss://orbit.example.com/api/v1/x");
        let local = ServerUrl::parse("http://127.0.0.1:18080").unwrap();
        assert_eq!(local.ws("/a"), "ws://127.0.0.1:18080/a");
        assert!(!local.is_secure());
    }

    #[test]
    fn paths_clear_text_and_junk_are_refused() {
        assert_eq!(ServerUrl::parse(""), Err(ServerUrlError::Empty));
        assert_eq!(ServerUrl::parse("http://orbit.example.com"), Err(ServerUrlError::Insecure));
        assert_eq!(ServerUrl::parse("https://orbit.example.com/chat"), Err(ServerUrlError::NotAnOrigin));
        assert_eq!(ServerUrl::parse("https://me@orbit.example.com"), Err(ServerUrlError::NotAnOrigin));
        assert_eq!(ServerUrl::parse("https://orbit.example.com?x=1"), Err(ServerUrlError::NotAnOrigin));
        assert_eq!(ServerUrl::parse("ftp://orbit.example.com"), Err(ServerUrlError::Invalid));
        assert_eq!(ServerUrl::parse("https://orbit.example.com:99999"), Err(ServerUrlError::Invalid));
        assert_eq!(ServerUrl::parse("https://or bit"), Err(ServerUrlError::Invalid));
    }
}
