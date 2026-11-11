use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, PoisonError, RwLock};

use axum::http::header::{
    CONTENT_SECURITY_POLICY, HeaderName, HeaderValue, REFERRER_POLICY, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, Method, Uri};
use ipnet::IpNet;

use super::RequestId;

const X_FORWARDED_FOR: &str = "x-forwarded-for";
const X_FORWARDED_PROTO: &str = "x-forwarded-proto";
const X_REQUEST_ID: &str = "x-request-id";

/// The client address after applying the explicitly trusted proxy policy.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ClientIp(pub IpAddr);

/// Canonical original request transport after applying trusted proxy headers.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RequestTransport {
    Http,
    Https,
}

impl RequestTransport {
    #[must_use]
    pub const fn is_secure(self) -> bool {
        matches!(self, Self::Https)
    }
}

/// Same-origin browser policy and explicitly trusted reverse-proxy networks.
#[derive(Clone, Debug)]
pub struct OriginPolicy {
    allowed_origins: BTreeSet<String>,
    allow_any_http_origin: bool,
    trusted_proxies: Vec<IpNet>,
}

impl OriginPolicy {
    #[must_use]
    pub fn new(application_origin: impl Into<String>) -> Self {
        let mut allowed_origins = BTreeSet::new();
        let application_origin = application_origin.into();
        allowed_origins.insert(application_origin);
        Self {
            allowed_origins,
            allow_any_http_origin: false,
            trusted_proxies: Vec::new(),
        }
    }

    #[must_use]
    pub fn allow_any_http_origin(mut self) -> Self {
        self.allow_any_http_origin = true;
        self
    }

    #[must_use]
    pub fn allow_origin(mut self, origin: impl Into<String>) -> Self {
        self.allowed_origins.insert(origin.into());
        self
    }

    #[must_use]
    pub fn trust_proxy(mut self, network: IpNet) -> Self {
        self.trusted_proxies.push(network);
        self
    }

    pub(crate) fn is_trusted_proxy(&self, peer: Option<IpAddr>) -> bool {
        peer.is_some_and(|ip| self.is_trusted_ip(ip))
    }

    fn is_trusted_ip(&self, ip: IpAddr) -> bool {
        self.trusted_proxies
            .iter()
            .any(|network| network.contains(&ip))
    }

    pub(crate) fn permits(&self, method: &Method, uri: &Uri, headers: &HeaderMap) -> bool {
        if matches!(
            uri.path(),
            "/api/public/v1/integrations/discord/events"
                | "/api/v1/integrations/github/webhook"
                | "/api/v1/billing/stripe/webhook"
                | "/oauth/register"
                | "/oauth/token"
                | "/oauth/revoke"
                | "/oauth/userinfo"
        ) || uri.path().starts_with("/api/v1/hooks/chat/")
            || uri.path().starts_with("/api/v1/hooks/bots/")
        {
            // Machine-to-machine: the secret in the path is the credential, and no cookie is read.
            return true;
        }
        if matches!(
            *method,
            Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
        ) && !headers.contains_key(axum::http::header::UPGRADE)
        {
            return true;
        }

        if is_ticket_socket(method, uri, headers) {
            return true;
        }

        let mut origins = headers.get_all(axum::http::header::ORIGIN).iter();
        let Some(origin) = origins.next() else {
            // Browsers send Origin on every POST and WebSocket upgrade, so a request without one
            // is not a browser. Without cookies it carries no ambient authority to forge: MCP
            // clients and native apps send a bearer secret, and native sign-in sends a password.
            // Present Origins still follow the normal policy and SDK validation.
            if uri.path() == "/mcp" {
                return true;
            }
            return !headers.contains_key(axum::http::header::COOKIE)
                && (bearer_token(headers).is_some()
                    || (*method == Method::POST
                        && matches!(
                            uri.path(),
                            "/api/v1/auth/login" | "/api/v1/auth/login/two-factor"
                        )));
        };
        origins.next().is_none()
            && origin.to_str().ok().is_some_and(|origin| {
                if self.allow_any_http_origin {
                    is_valid_http_origin(origin)
                } else {
                    is_valid_http_origin(origin) && self.allowed_origins.contains(origin)
                }
            })
    }
}

/// A co-editing socket opened with a socket ticket and no cookie, from any or no Origin: a
/// browser socket in a native app's WebView sends the WebView's Origin and cannot send the
/// session. The ticket is a short-lived secret the page obtained with the session, so the
/// request carries no ambient authority to forge; the socket verifies it.
fn is_ticket_socket(method: &Method, uri: &Uri, headers: &HeaderMap) -> bool {
    *method == Method::GET
        && headers.contains_key(axum::http::header::UPGRADE)
        && !headers.contains_key(axum::http::header::COOKIE)
        && uri.path().ends_with("/collab")
        && uri
            .query()
            .is_some_and(|query| query.split('&').any(|pair| pair.starts_with("ticket=")))
}

/// The token of the request's single `Authorization: Bearer` header. `None` when the header is
/// missing, repeated, of another scheme, or its token is empty or contains whitespace.
#[must_use]
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(axum::http::header::AUTHORIZATION).iter();
    let value = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, token) = value.to_str().ok()?.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer")
        && !token.is_empty()
        && !token.chars().any(char::is_whitespace))
    .then_some(token)
}

pub(crate) struct RequestSecurity {
    pub client_ip: ClientIp,
    pub request_id: RequestId,
    pub transport: RequestTransport,
}

pub(crate) fn inspect_request(
    headers: &HeaderMap,
    peer: Option<IpAddr>,
    policy: &OriginPolicy,
) -> Result<RequestSecurity, ()> {
    let trusted = policy.is_trusted_proxy(peer);
    let mut client_ip = peer.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let mut request_id = RequestId::new();
    // The composed listener is plain TCP. Request-target text is not connection metadata and an
    // absolute-form `https://` URI must never be accepted as proof of TLS.
    let mut transport = RequestTransport::Http;

    if trusted {
        let chain = forwarded_for(headers)?;
        if !chain.is_empty() {
            let first = *chain.first().ok_or(())?;
            client_ip = chain
                .iter()
                .rev()
                .copied()
                .find(|ip| !policy.is_trusted_ip(*ip))
                .unwrap_or(first);
        }
        // Proxies pass the client's `X-Request-Id` on unchanged, so it is untrusted input: a value
        // Orbit cannot use gets a server-generated ID instead of failing the request (issue #52).
        if let Some(forwarded) = consistent_header(headers, X_REQUEST_ID)
            .ok()
            .flatten()
            .and_then(|value| value.to_str().ok())
            .and_then(RequestId::from_trusted_header)
        {
            request_id = forwarded;
        }
        if let Some(forwarded_transport) = forwarded_transport(headers)? {
            transport = forwarded_transport;
        }
    }

    Ok(RequestSecurity {
        client_ip: ClientIp(client_ip),
        request_id,
        transport,
    })
}

pub(crate) fn strip_forwarding_headers(headers: &mut HeaderMap) {
    headers.remove(X_REQUEST_ID);
    headers.remove(X_FORWARDED_FOR);
    headers.remove(X_FORWARDED_PROTO);
}

fn consistent_header<'a>(
    headers: &'a HeaderMap,
    name: &str,
) -> Result<Option<&'a HeaderValue>, ()> {
    let mut values = headers.get_all(name).iter();
    let value = values.next();
    if value.is_some_and(|value| values.any(|candidate| candidate != value)) {
        return Err(());
    }
    Ok(value)
}

fn forwarded_for(headers: &HeaderMap) -> Result<Vec<IpAddr>, ()> {
    headers
        .get_all(X_FORWARDED_FOR)
        .iter()
        .map(|value| value.to_str().map_err(|_| ()))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flat_map(|value| value.split(',').map(str::trim))
        .map(|value| value.parse().map_err(|_| ()))
        .collect()
}

fn forwarded_transport(headers: &HeaderMap) -> Result<Option<RequestTransport>, ()> {
    let mut transport = None;
    for value in headers.get_all(X_FORWARDED_PROTO) {
        for candidate in value.to_str().map_err(|_| ())?.split(',').map(str::trim) {
            let candidate = match candidate {
                "https" | "wss" => RequestTransport::Https,
                "http" | "ws" => RequestTransport::Http,
                _ => return Err(()),
            };
            if transport.is_some_and(|transport| transport != candidate) {
                return Err(());
            }
            transport = Some(candidate);
        }
    }
    Ok(transport)
}

/// The origins of the configured object storage buckets that pages may reach (Drive's presigned URLs): only
/// `https://host[:port]` origins. Cloned handles share one list, which the storage settings replace at run time.
#[derive(Clone, Debug, Default)]
pub struct CspSources(Arc<RwLock<String>>);

impl CspSources {
    /// Replaces the list. Anything but an `https` origin of plain host characters is left out, so a setting can
    /// neither allow plain http nor add other directives.
    pub fn set(&self, origins: impl IntoIterator<Item = String>) {
        let mut kept: Vec<String> = origins
            .into_iter()
            .filter(|origin| {
                origin.strip_prefix("https://").is_some_and(|authority| {
                    !authority.is_empty()
                        && authority.chars().all(|c| {
                            c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']')
                        })
                })
            })
            .map(|origin| origin.to_ascii_lowercase())
            .collect();
        kept.sort();
        kept.dedup();
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = kept.join(" ");
    }

    /// The origins, separated by spaces.
    #[must_use]
    pub fn get(&self) -> String {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// What the Content Security Policy of a response depends on.
#[derive(Clone, Debug, Default)]
pub(crate) struct Csp {
    pub(crate) script_hash: Option<String>,
    pub(crate) sources: CspSources,
}

pub(crate) fn add_security_headers(headers: &mut HeaderMap, secure: bool, csp: &Csp) {
    let script_source = csp
        .script_hash
        .as_deref()
        .map(|hash| format!("script-src 'self' '{hash}'; "))
        .unwrap_or_else(|| "script-src 'self'; ".to_owned());
    // Drive's presigned URLs: fetches, images and media of the configured buckets only. External https images
    // (docs covers, image blocks, link previews) and X videos in chat were allowed before Drive and stay.
    let buckets = csp.sources.get();
    let buckets = if buckets.is_empty() {
        String::new()
    } else {
        format!(" {buckets}")
    };
    // The style-src hash is the empty string's: it allows only an empty `<style>`, which BlockNote's placeholder
    // plugin creates and then fills through the CSSOM (`insertRule`), which CSP does not restrict.
    let policy = format!(
        "default-src 'self'; {script_source}style-src 'self' 'sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU='; style-src-attr 'unsafe-inline'; img-src 'self' data: blob: https:{buckets}; connect-src 'self'{buckets}; frame-src 'self' https://www.youtube-nocookie.com; media-src 'self' blob: https://video.twimg.com{buckets}; frame-ancestors 'none'; object-src 'none'; base-uri 'none'; form-action 'self' https://github.com"
    );
    // A response may carry its own policy (the wrapper document of a workspace app, which Orbit
    // frames itself). Its `frame-ancestors` decides framing then, so it gets no `X-Frame-Options`.
    if headers.contains_key(CONTENT_SECURITY_POLICY) {
        headers.remove(X_FRAME_OPTIONS);
    } else {
        headers.insert(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_str(&policy).expect("validated CSP values form a valid header"),
        );
        // For browsers without CSP `frame-ancestors`.
        headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    }
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    // No other site keeps a handle to an Orbit window. `allow-popups`: Orbit keeps its handle to
    // the app sign-in popup (`signInPopup.ts` watches it close), and apps framed in Orbit can open
    // their own sign-in tabs; `same-origin` broke both in Chromium.
    headers.insert(
        HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin-allow-popups"),
    );
    // A response may also set a stricter one (the framed `login_required` page of app sign-in).
    if !headers.contains_key(REFERRER_POLICY) {
        headers.insert(
            REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        );
    }
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    if secure {
        headers.insert(
            HeaderName::from_static("strict-transport-security"),
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
}

fn is_valid_http_origin(origin: &str) -> bool {
    let Ok(uri) = origin.parse::<Uri>() else {
        return false;
    };
    matches!(uri.scheme_str(), Some("http" | "https"))
        && uri.authority().is_some_and(|authority| {
            let host = authority.host();
            let port = &authority.as_str()[host.len()..];
            !host.is_empty()
                && !authority.as_str().contains('@')
                && (port.is_empty()
                    || port
                        .strip_prefix(':')
                        .is_some_and(|port| !port.is_empty() && port.parse::<u16>().is_ok()))
        })
        && uri.path() == "/"
        && uri.query().is_none()
}

#[cfg(test)]
mod tests {
    use axum::http::header::{AUTHORIZATION, COOKIE, ORIGIN, UPGRADE};
    use axum::http::header::{CONTENT_SECURITY_POLICY, REFERRER_POLICY, X_FRAME_OPTIONS};
    use axum::http::{HeaderMap, HeaderValue, Method, Uri};

    use super::{Csp, CspSources, OriginPolicy, add_security_headers, bearer_token};

    #[test]
    fn responses_refuse_framing_and_isolate_the_opener() {
        let mut headers = HeaderMap::new();
        add_security_headers(&mut headers, true, &Csp::default());
        assert_eq!(headers["x-frame-options"], "DENY");
        // Without a bucket, pages fetch only from Orbit itself.
        let csp = headers[CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(csp.contains("connect-src 'self';"));
        assert!(csp.contains("media-src 'self' blob: https://video.twimg.com;"));
        assert_eq!(
            headers["cross-origin-opener-policy"],
            "same-origin-allow-popups"
        );
    }

    fn policy(sources: &CspSources) -> String {
        let mut headers = HeaderMap::new();
        add_security_headers(
            &mut headers,
            true,
            &Csp {
                script_hash: None,
                sources: sources.clone(),
            },
        );
        headers[CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn only_https_bucket_origins_reach_fetches_images_and_media() {
        let sources = CspSources::default();
        sources.set([
            "https://ACCOUNT.r2.cloudflarestorage.com".to_owned(),
            "https://files.example.com:8443".to_owned(),
            "http://minio.local:9000".to_owned(),
            "https://evil.example.com; script-src *".to_owned(),
            "https://".to_owned(),
            "https://files.example.com:8443".to_owned(),
        ]);
        let csp = policy(&sources);
        let allowed = "https://account.r2.cloudflarestorage.com https://files.example.com:8443";
        assert!(
            csp.contains(&format!("connect-src 'self' {allowed};")),
            "{csp}"
        );
        assert!(
            csp.contains(&format!(
                "media-src 'self' blob: https://video.twimg.com {allowed};"
            )),
            "{csp}"
        );
        assert!(
            csp.contains(&format!("img-src 'self' data: blob: https: {allowed};")),
            "{csp}"
        );
        assert!(!csp.contains("http:"), "plain http stays refused: {csp}");
        assert!(!csp.contains("minio") && !csp.contains("evil"), "{csp}");
        assert!(!csp.contains("connect-src 'self' https:;") && !csp.contains("connect-src *"));
    }

    #[test]
    fn a_new_bucket_origin_applies_to_the_next_response_without_a_restart() {
        let sources = CspSources::default();
        let layer_copy = sources.clone();
        assert!(policy(&layer_copy).contains("connect-src 'self';"));
        sources.set(["https://new.example.com".to_owned()]);
        assert!(policy(&layer_copy).contains("connect-src 'self' https://new.example.com;"));
        sources.set([]);
        assert!(!policy(&layer_copy).contains("new.example.com"));
    }

    #[test]
    fn a_response_with_its_own_policy_decides_its_framing() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("frame-ancestors 'self'"),
        );
        headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
        add_security_headers(&mut headers, true, &Csp::default());
        assert_eq!(headers[CONTENT_SECURITY_POLICY], "frame-ancestors 'self'");
        assert!(!headers.contains_key(X_FRAME_OPTIONS));
        assert_eq!(headers[REFERRER_POLICY], "no-referrer");
    }

    fn permits(method: Method, path: &str, headers: &[(axum::http::HeaderName, &str)]) -> bool {
        let mut map = HeaderMap::new();
        for (name, value) in headers {
            map.append(name, value.parse().unwrap());
        }
        OriginPolicy::new("https://orbit.test").permits(
            &method,
            &path.parse::<Uri>().unwrap(),
            &map,
        )
    }

    const TASKS: &str = "/api/v1/workspaces/w/tasks";

    #[test]
    fn bearer_without_origin_or_cookie_is_allowed() {
        assert!(permits(
            Method::POST,
            TASKS,
            &[(AUTHORIZATION, "Bearer secret")]
        ));
        assert!(permits(
            Method::GET,
            "/api/v1/live",
            &[(AUTHORIZATION, "bearer secret"), (UPGRADE, "websocket")]
        ));
    }

    #[test]
    fn bearer_without_origin_but_with_a_cookie_is_rejected() {
        assert!(!permits(
            Method::POST,
            TASKS,
            &[
                (AUTHORIZATION, "Bearer secret"),
                (COOKIE, "__Host-orbit_session=s")
            ]
        ));
    }

    #[test]
    fn bearer_with_a_foreign_origin_is_rejected() {
        assert!(!permits(
            Method::POST,
            TASKS,
            &[
                (AUTHORIZATION, "Bearer secret"),
                (ORIGIN, "https://evil.example")
            ]
        ));
    }

    #[test]
    fn native_sign_in_without_origin_or_cookie_is_allowed() {
        assert!(permits(Method::POST, "/api/v1/auth/login", &[]));
        assert!(permits(Method::POST, "/api/v1/auth/login/two-factor", &[]));
        assert!(!permits(
            Method::POST,
            "/api/v1/auth/login",
            &[(COOKIE, "__Host-orbit_session=s")]
        ));
    }

    #[test]
    fn other_mutations_without_origin_are_rejected() {
        assert!(!permits(Method::POST, "/api/v1/auth/recovery/request", &[]));
        assert!(!permits(
            Method::POST,
            TASKS,
            &[(AUTHORIZATION, "Basic abc")]
        ));
    }

    const COLLAB: &str = "/api/v1/workspaces/w/pages/p/collab?v=1&epoch=e&ticket=t";

    #[test]
    fn ticket_sockets_are_allowed_from_any_or_no_origin_without_a_cookie() {
        for origin in [None, Some("null"), Some("https://evil.example")] {
            let mut headers = vec![(UPGRADE, "websocket")];
            headers.extend(origin.map(|origin| (ORIGIN, origin)));
            assert!(permits(Method::GET, COLLAB, &headers), "{origin:?}");
        }
        assert!(permits(
            Method::GET,
            "/api/v1/workspaces/w/tasks/t/collab?ticket=t&v=1",
            &[(UPGRADE, "websocket"), (ORIGIN, "file://")]
        ));
    }

    #[test]
    fn ticket_sockets_keep_the_origin_rules_otherwise() {
        let foreign = (ORIGIN, "https://evil.example");
        let upgrade = (UPGRADE, "websocket");
        // With a cookie.
        assert!(!permits(
            Method::GET,
            COLLAB,
            &[
                upgrade.clone(),
                foreign.clone(),
                (COOKIE, "orbit_session=s")
            ]
        ));
        // Without a ticket, or with a lookalike parameter.
        for path in [
            "/api/v1/workspaces/w/pages/p/collab?v=1&epoch=e",
            "/api/v1/workspaces/w/pages/p/collab?v=1&xticket=t",
            "/api/v1/workspaces/w/pages/p/collab",
        ] {
            assert!(
                !permits(Method::GET, path, &[upgrade.clone(), foreign.clone()]),
                "{path}"
            );
        }
        // Another socket or a plain request.
        assert!(!permits(
            Method::GET,
            "/api/v1/live?ticket=t",
            &[upgrade.clone(), foreign.clone()]
        ));
        assert!(!permits(
            Method::GET,
            "/api/v1/workspaces/w/pages/p/collab/x?ticket=t",
            &[upgrade.clone(), foreign.clone()]
        ));
        assert!(!permits(
            Method::POST,
            COLLAB,
            &[(ORIGIN, "https://evil.example")]
        ));
        assert!(!permits(Method::POST, COLLAB, &[upgrade, foreign]));
    }

    #[test]
    fn bearer_token_takes_exactly_one_well_formed_header() {
        let headers = |values: &[&str]| {
            let mut map = HeaderMap::new();
            for value in values {
                map.append(AUTHORIZATION, value.parse().unwrap());
            }
            map
        };
        assert_eq!(bearer_token(&headers(&["Bearer abc"])), Some("abc"));
        assert_eq!(bearer_token(&headers(&["BEARER abc"])), Some("abc"));
        assert_eq!(bearer_token(&headers(&["Bearer "])), None);
        assert_eq!(bearer_token(&headers(&["Bearer a b"])), None);
        assert_eq!(bearer_token(&headers(&["Basic abc"])), None);
        assert_eq!(bearer_token(&headers(&["Bearer abc", "Bearer abc"])), None);
        assert_eq!(bearer_token(&headers(&[])), None);
    }
}
