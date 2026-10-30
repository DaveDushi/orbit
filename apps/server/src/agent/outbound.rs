//! Orbit's calls to tools outside Orbit for the agent (`super::integrations`): a remote MCP server
//! (Streamable HTTP) or an HTTP API. Only HTTPS to public addresses: every address a host name
//! resolves to must be public, IP literals are checked, redirects are not followed and no proxy is
//! used. Answers are cut at a size limit and every call has a timeout. Hosts in
//! `ORBIT_AGENT_INTEGRATION_TEST_HOSTS` skip the address check and may use plain HTTP: for local
//! tests only, never in production.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use orbit_platform::is_public_ip;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::{Value, json};
use url::{Host, Url};

/// Whole call, including the answer.
const TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The most Orbit reads of one answer.
pub(crate) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

#[derive(Debug, thiserror::Error)]
pub(crate) enum OutboundError {
    #[error("{0}")]
    Refused(&'static str),
    #[error("the service could not be reached: {0}")]
    Network(String),
    #[error("the service answered HTTP {0}")]
    Status(u16),
    #[error("the answer is larger than 1 MiB")]
    TooLarge,
    #[error("the MCP server answered in an unexpected way: {0}")]
    Protocol(String),
    #[error("the MCP server returned an error: {0}")]
    Remote(String),
}

/// The hosts that skip the address check (lower case, no port).
#[derive(Clone, Debug, Default)]
pub(crate) struct TestHosts(Arc<[String]>);

impl TestHosts {
    /// `ORBIT_AGENT_INTEGRATION_TEST_HOSTS`: a comma-separated list of host names or IPs.
    pub(crate) fn from_env() -> Self {
        Self::new(
            std::env::var("ORBIT_AGENT_INTEGRATION_TEST_HOSTS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|host| !host.is_empty()),
        )
    }

    pub(crate) fn new<'a>(hosts: impl IntoIterator<Item = &'a str>) -> Self {
        let hosts: Vec<String> = hosts.into_iter().map(str::to_ascii_lowercase).collect();
        if !hosts.is_empty() {
            tracing::warn!(
                ?hosts,
                "ORBIT_AGENT_INTEGRATION_TEST_HOSTS is set: agent integrations may reach these hosts \
                 over plain HTTP and on local addresses. Never set it in production."
            );
        }
        Self(hosts.into())
    }

    fn contains(&self, host: &str) -> bool {
        let host = host.trim_start_matches('[').trim_end_matches(']');
        self.0
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    }
}

/// Checks a URL an admin entered: HTTPS (HTTP only for test hosts), no user name or password, no
/// fragment, and an IP literal must be public. Host names are checked when they resolve.
pub(crate) fn check_url(raw: &str, test_hosts: &TestHosts) -> Result<Url, OutboundError> {
    let url =
        Url::parse(raw.trim()).map_err(|_| OutboundError::Refused("The URL is not valid."))?;
    let host = url
        .host_str()
        .ok_or(OutboundError::Refused("The URL has no host."))?;
    let test = test_hosts.contains(host);
    match url.scheme() {
        "https" => {}
        "http" if test => {}
        _ => return Err(OutboundError::Refused("The URL must start with https://.")),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(OutboundError::Refused(
            "Put credentials in the secret, not in the URL.",
        ));
    }
    if url.fragment().is_some() {
        return Err(OutboundError::Refused("The URL must not have a #fragment."));
    }
    let literal = match url.host() {
        Some(Host::Ipv4(ip)) => Some(IpAddr::V4(ip)),
        Some(Host::Ipv6(ip)) => Some(IpAddr::V6(ip)),
        _ => None,
    };
    if let Some(ip) = literal
        && !test
        && !is_public_ip(ip)
    {
        return Err(OutboundError::Refused(
            "The URL points to a local or private address.",
        ));
    }
    Ok(url)
}

/// `base` joined with the agent's `path` and `query`, only when the result stays below `base`:
/// same scheme, host and port, and a path that starts with the base path. Paths with `..` or `.`
/// segments, encoded `.`, `/` or `\`, a scheme, `//`, `?` or `#` are refused before joining.
pub(crate) fn join_api_url(
    base: &Url,
    path: &str,
    query: &[(String, String)],
) -> Result<Url, OutboundError> {
    let refuse = || OutboundError::Refused("The path must stay below the API's base URL.");
    if !path.starts_with('/') || path.starts_with("//") || path.len() > 2_048 {
        return Err(refuse());
    }
    let lower = path.to_ascii_lowercase();
    if path.contains(['?', '#', '\\'])
        || path.chars().any(char::is_control)
        || lower.contains("://")
        || ["%2e", "%2f", "%5c", "%00"]
            .iter()
            .any(|encoded| lower.contains(encoded))
        || path
            .split('/')
            .any(|segment| segment == ".." || segment == ".")
    {
        return Err(refuse());
    }
    let base_path = base.path().trim_end_matches('/');
    let mut url = base.clone();
    url.set_query(None);
    url.set_path(&format!("{base_path}{path}"));
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    let inside = url.scheme() == base.scheme()
        && url.host() == base.host()
        && url.port_or_known_default() == base.port_or_known_default()
        && (url.path() == base_path
            || url.path().starts_with(&format!("{base_path}/"))
            || base_path.is_empty());
    if inside { Ok(url) } else { Err(refuse()) }
}

/// Resolves a host and refuses it when any of its addresses is not public (test hosts excepted).
struct GuardedResolver(TestHosts);

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let test = self.0.contains(name.as_str());
        Box::pin(async move {
            let addresses: Vec<SocketAddr> =
                tokio::net::lookup_host((name.as_str(), 0)).await?.collect();
            if addresses.is_empty() {
                return Err("the host has no address".into());
            }
            if !test && addresses.iter().any(|address| !is_public_ip(address.ip())) {
                return Err("the host resolves to a local or private address".into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

/// The credential header of one integration, opened just for the call.
pub(crate) struct Credential {
    pub name: String,
    pub value: String,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Credential")
            .field("name", &self.name)
            .field("value", &"[REDACTED]")
            .finish()
    }
}

/// The answer of an HTTP API, as the agent gets it.
#[derive(Debug, serde::Serialize)]
pub(crate) struct ApiAnswer {
    pub status: u16,
    pub content_type: Option<String>,
    /// JSON when the answer is JSON, else text (cut at the size limit).
    pub body: Value,
    pub truncated: bool,
}

#[derive(Clone)]
pub(crate) struct Outbound {
    client: reqwest::Client,
    test_hosts: TestHosts,
}

impl Outbound {
    pub(crate) fn new(test_hosts: TestHosts) -> Self {
        let client = reqwest::Client::builder()
            .user_agent(concat!("Orbit-Agent/", env!("CARGO_PKG_VERSION")))
            .timeout(TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            // A proxy would resolve the host itself, past the address check.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(GuardedResolver(test_hosts.clone())))
            .build()
            .expect("the integration client builds");
        Self { client, test_hosts }
    }

    pub(crate) fn test_hosts(&self) -> &TestHosts {
        &self.test_hosts
    }

    fn headers(credential: Option<&Credential>) -> Result<HeaderMap, OutboundError> {
        let mut headers = HeaderMap::new();
        if let Some(credential) = credential {
            let name = HeaderName::from_bytes(credential.name.as_bytes())
                .map_err(|_| OutboundError::Refused("The credential header name is not valid."))?;
            let mut value = HeaderValue::from_str(&credential.value).map_err(|_| {
                OutboundError::Refused("The credential is not a valid header value.")
            })?;
            value.set_sensitive(true);
            headers.insert(name, value);
        }
        Ok(headers)
    }

    async fn send(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, OutboundError> {
        request.send().await.map_err(|error| {
            // Without the URL: it could carry a key in its query.
            OutboundError::Network(network_reason(&error.without_url()))
        })
    }

    /// One request to an HTTP API, checked again before it leaves.
    pub(crate) async fn api_request(
        &self,
        url: &Url,
        method: reqwest::Method,
        body: Option<&Value>,
        credential: Option<&Credential>,
    ) -> Result<ApiAnswer, OutboundError> {
        let url = check_url(url.as_str(), &self.test_hosts)?;
        let mut request = self
            .client
            .request(method, url)
            .headers(Self::headers(credential)?)
            .header("accept", "application/json, text/plain;q=0.9, */*;q=0.1");
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = self.send(request).await?;
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let (bytes, truncated) = read_limited(response, false).await?;
        let body = if content_type
            .as_deref()
            .is_some_and(|kind| kind.contains("json"))
            && !truncated
        {
            serde_json::from_slice(&bytes)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()))
        } else {
            Value::String(String::from_utf8_lossy(&bytes).into_owned())
        };
        Ok(ApiAnswer {
            status,
            content_type,
            body,
            truncated,
        })
    }

    /// The tools of an MCP server (`tools/list`, all pages up to 20).
    pub(crate) async fn mcp_tools(
        &self,
        url: &Url,
        credential: Option<&Credential>,
    ) -> Result<Vec<Value>, OutboundError> {
        let mut session = self.mcp_open(url, credential).await?;
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for page in 0..20 {
            let params = cursor
                .as_ref()
                .map_or_else(|| json!({}), |cursor| json!({ "cursor": cursor }));
            let result = session.request(self, 2 + page, "tools/list", params).await;
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    session.close(self).await;
                    return Err(error);
                }
            };
            if let Some(list) = result.get("tools").and_then(Value::as_array) {
                tools.extend(list.iter().cloned());
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        session.close(self).await;
        Ok(tools)
    }

    /// One `tools/call` on an MCP server; the result as the server sent it.
    pub(crate) async fn mcp_call(
        &self,
        url: &Url,
        credential: Option<&Credential>,
        tool: &str,
        arguments: &Value,
    ) -> Result<Value, OutboundError> {
        let mut session = self.mcp_open(url, credential).await?;
        let result = session
            .request(
                self,
                2,
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
            )
            .await;
        session.close(self).await;
        result
    }

    /// `initialize` and `notifications/initialized`: a session that lives for one operation.
    async fn mcp_open<'a>(
        &self,
        url: &'a Url,
        credential: Option<&'a Credential>,
    ) -> Result<McpSession<'a>, OutboundError> {
        let url = check_url(url.as_str(), &self.test_hosts)?;
        let mut session = McpSession {
            url,
            credential,
            id: None,
            version: None,
        };
        let initialized = session
            .request(
                self,
                1,
                "initialize",
                json!({
                    "protocolVersion": MCP_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "orbit-agent", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await?;
        session.version = initialized
            .get("protocolVersion")
            .and_then(Value::as_str)
            .map(str::to_owned);
        session.notify(self, "notifications/initialized").await?;
        Ok(session)
    }
}

struct McpSession<'a> {
    url: Url,
    credential: Option<&'a Credential>,
    /// `Mcp-Session-Id` from the server, when it keeps sessions.
    id: Option<String>,
    version: Option<String>,
}

impl McpSession<'_> {
    fn post(
        &self,
        outbound: &Outbound,
        message: &Value,
    ) -> Result<reqwest::RequestBuilder, OutboundError> {
        let mut request = outbound
            .client
            .post(self.url.clone())
            .headers(Outbound::headers(self.credential)?)
            .header("accept", "application/json, text/event-stream")
            .json(message);
        if let Some(id) = &self.id {
            request = request.header("mcp-session-id", id);
        }
        if let Some(version) = &self.version {
            request = request.header("mcp-protocol-version", version);
        }
        Ok(request)
    }

    async fn request(
        &mut self,
        outbound: &Outbound,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, OutboundError> {
        let message = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let response = outbound.send(self.post(outbound, &message)?).await?;
        if !response.status().is_success() {
            return Err(OutboundError::Status(response.status().as_u16()));
        }
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
        {
            self.id = Some(session.to_owned());
        }
        let stream = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|kind| kind.starts_with("text/event-stream"));
        let reply = if stream {
            read_sse_reply(response, id).await?
        } else {
            let (bytes, _) = read_limited(response, true).await?;
            serde_json::from_slice::<Value>(&bytes)
                .map_err(|_| OutboundError::Protocol("the answer is not JSON".to_owned()))?
        };
        rpc_result(reply, id)
    }

    async fn notify(&self, outbound: &Outbound, method: &str) -> Result<(), OutboundError> {
        let message = json!({ "jsonrpc": "2.0", "method": method });
        let response = outbound.send(self.post(outbound, &message)?).await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(OutboundError::Status(response.status().as_u16()))
        }
    }

    /// Ends a server-side session; a failure changes nothing for the call.
    async fn close(&self, outbound: &Outbound) {
        let Some(id) = &self.id else { return };
        let Ok(headers) = Outbound::headers(self.credential) else {
            return;
        };
        let _ = outbound
            .client
            .delete(self.url.clone())
            .headers(headers)
            .header("mcp-session-id", id)
            .timeout(Duration::from_secs(5))
            .send()
            .await;
    }
}

/// The `result` of a JSON-RPC answer to request `id` (a batch is searched for it).
fn rpc_result(reply: Value, id: u64) -> Result<Value, OutboundError> {
    let reply = match reply {
        Value::Array(items) => items
            .into_iter()
            .find(|item| item.get("id").and_then(Value::as_u64) == Some(id))
            .ok_or_else(|| OutboundError::Protocol("no answer to the request".to_owned()))?,
        reply => reply,
    };
    if let Some(error) = reply.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("error");
        return Err(OutboundError::Remote(message.chars().take(300).collect()));
    }
    reply
        .get("result")
        .cloned()
        .ok_or_else(|| OutboundError::Protocol("the answer has no result".to_owned()))
}

/// Reads an SSE answer until the JSON-RPC reply to `id` arrives (other events are skipped).
async fn read_sse_reply(mut response: reqwest::Response, id: u64) -> Result<Value, OutboundError> {
    let mut buffer = Vec::new();
    let mut read = 0;
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|error| OutboundError::Network(network_reason(&error.without_url())))?;
        let Some(chunk) = chunk else {
            return Err(OutboundError::Protocol(
                "the event stream ended without an answer".to_owned(),
            ));
        };
        read += chunk.len();
        if read > MAX_RESPONSE_BYTES {
            return Err(OutboundError::TooLarge);
        }
        // `\r` is one byte in UTF-8, so dropping it never splits a character.
        buffer.extend(chunk.iter().copied().filter(|byte| *byte != b'\r'));
        let Some(end) = buffer.windows(2).rposition(|pair| pair == b"\n\n") else {
            continue;
        };
        let complete: Vec<u8> = buffer.drain(..end + 2).collect();
        let text = String::from_utf8_lossy(&complete);
        for event in text.split("\n\n") {
            let data: Vec<&str> = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
                .collect();
            if data.is_empty() {
                continue;
            }
            let Ok(message) = serde_json::from_str::<Value>(&data.join("\n")) else {
                continue;
            };
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(message);
            }
        }
    }
}

/// The body up to the size limit; `strict` fails beyond it, else the rest is dropped.
async fn read_limited(
    mut response: reqwest::Response,
    strict: bool,
) -> Result<(Vec<u8>, bool), OutboundError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| OutboundError::Network(network_reason(&error.without_url())))?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            if strict {
                return Err(OutboundError::TooLarge);
            }
            let room = MAX_RESPONSE_BYTES - bytes.len();
            bytes.extend_from_slice(&chunk[..room]);
            return Ok((bytes, true));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, false))
}

fn network_reason(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        return "timeout".to_owned();
    }
    // The innermost cause names the reason (refused address, DNS, TLS) without request data.
    let mut source: &dyn std::error::Error = error;
    while let Some(next) = source.source() {
        source = next;
    }
    source.to_string().chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use super::{TestHosts, check_url, join_api_url};

    fn base(raw: &str) -> url::Url {
        url::Url::parse(raw).unwrap()
    }

    #[test]
    fn only_https_urls_to_public_hosts_are_accepted() {
        let none = TestHosts::default();
        for good in [
            "https://api.cloudflare.com/client/v4",
            "https://mcp.example.com/mcp",
            "https://1.1.1.1/",
            "https://[2606:4700::1111]/",
        ] {
            assert!(check_url(good, &none).is_ok(), "{good}");
        }
        for bad in [
            "http://api.example.com/",
            "ftp://example.com/",
            "file:///etc/passwd",
            "https://user:pass@example.com/",
            "https://example.com/#x",
            "https://127.0.0.1/",
            "https://10.0.0.8/",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/",
            "https://[::ffff:127.0.0.1]/",
            "https://[fd00::1]/",
            "https://100.64.0.1/",
            "not a url",
        ] {
            assert!(check_url(bad, &none).is_err(), "{bad}");
        }
    }

    #[test]
    fn test_hosts_may_use_http_and_local_addresses() {
        let hosts = TestHosts::new(["127.0.0.1", "localhost"]);
        assert!(check_url("http://127.0.0.1:4000/mcp", &hosts).is_ok());
        assert!(check_url("http://localhost:4000/", &hosts).is_ok());
        assert!(check_url("http://10.0.0.1/", &hosts).is_err());
        assert!(check_url("ftp://127.0.0.1/", &hosts).is_err());
    }

    #[test]
    fn api_paths_stay_below_the_base_url() {
        let api = base("https://api.example.com/client/v4/");
        let joined = join_api_url(
            &api,
            "/zones/abc/dns_records",
            &[("page".into(), "2".into())],
        )
        .unwrap();
        assert_eq!(
            joined.as_str(),
            "https://api.example.com/client/v4/zones/abc/dns_records?page=2"
        );
        assert_eq!(
            join_api_url(&api, "/a b", &[]).unwrap().as_str(),
            "https://api.example.com/client/v4/a%20b"
        );
        for escape in [
            "zones",
            "//evil.example.com/x",
            "/../admin",
            "/zones/../../admin",
            "/./x",
            "/%2e%2e/admin",
            "/%2E%2E/admin",
            "/a%2fb",
            "/a\\..\\b",
            "/x?token=1",
            "/x#y",
            "/https://evil.example.com",
            "/a\u{0}b",
            "/a\nb",
        ] {
            assert!(join_api_url(&api, escape, &[]).is_err(), "{escape:?}");
        }
        // Query values are encoded, never parsed as a new URL.
        let joined = join_api_url(
            &api,
            "/x",
            &[("next".into(), "https://evil.example.com/&a=b".into())],
        )
        .unwrap();
        assert_eq!(joined.host_str(), Some("api.example.com"));
        assert!(joined.path().starts_with("/client/v4/"));

        let root = base("https://api.example.com");
        assert_eq!(
            join_api_url(&root, "/v1/me", &[]).unwrap().as_str(),
            "https://api.example.com/v1/me"
        );
    }
}
