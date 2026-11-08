//! The exe.dev auth proxy in front of a hosted Orbit. The app sends the person's exe.dev VM token
//! in [`GATEWAY_TOKEN_HEADER`]; the proxy strips it before Orbit (https://exe.dev/docs/https-tokens-for-vms.md).
//!
//! What the proxy answers (seen on orbit.wellrox.com, 2026-11-08, HTTP and WebSocket upgrades alike):
//! - no token: `307` to `/__exe.dev/login?…` (its browser sign-in);
//! - a token it refuses (malformed, expired, revoked): `401`, `text/plain`, `WWW-Authenticate: Bearer, Basic`.
//!
//! Orbit's own `401` is `application/problem+json` with no `WWW-Authenticate`, so the two differ.
//! The proxy also reads a plain `Authorization: Bearer` as an exe.dev token (its deprecated form):
//! without a valid gateway token, Orbit's session header alone gets the proxy's `401`.

/// The header that carries the exe.dev token.
pub const GATEWAY_TOKEN_HEADER: &str = "X-Exedev-Authorization";

/// Who answered a response, as far as the gateway goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatewayReply {
    /// The request reached Orbit (or there is no gateway); read the response as Orbit's.
    Passed,
    /// The proxy wants its browser sign-in: the app sent no token.
    SignInRequired,
    /// The proxy refused the token: ask the person for a new one.
    TokenRejected,
}

/// Classifies a response by its status and the `Location` and `WWW-Authenticate` headers.
pub fn classify(status: u16, location: Option<&str>, www_authenticate: Option<&str>) -> GatewayReply {
    let to_login = location.is_some_and(|l| l.contains("/__exe.dev/login"));
    if (300..400).contains(&status) && to_login {
        return GatewayReply::SignInRequired;
    }
    let proxy_challenge = www_authenticate.is_some_and(|w| {
        w.split(',').any(|scheme| scheme.trim().eq_ignore_ascii_case("bearer"))
    });
    if status == 401 && proxy_challenge {
        return GatewayReply::TokenRejected;
    }
    GatewayReply::Passed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proxy_replies_are_told_apart_from_orbits() {
        let login = "https://orbit.example/__exe.dev/login?exedev_host=orbit.example&redirect=%2Fapi%2Fv1%2Fsetup%2Fstatus";
        assert_eq!(classify(307, Some(login), None), GatewayReply::SignInRequired);
        assert_eq!(classify(401, None, Some("Bearer, Basic")), GatewayReply::TokenRejected);
        // Orbit's own 401 (problem+json, no challenge) and its redirects belong to Orbit.
        assert_eq!(classify(401, None, None), GatewayReply::Passed);
        assert_eq!(classify(307, Some("/chat"), None), GatewayReply::Passed);
        assert_eq!(classify(200, None, None), GatewayReply::Passed);
    }
}
