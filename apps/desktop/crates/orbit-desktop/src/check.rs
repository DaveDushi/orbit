//! `orbit-desktop --check-gateway <server>`: answers risk 4b on a real server. With valid
//! gateway cookies, does the exe.dev proxy forward `Authorization: Bearer` to Orbit, on HTTP and
//! on the WebSocket upgrades?
//!
//! It needs a person (or saved credentials): the sign-in window for the gateway, then the Orbit
//! email, password and, if on, a two-factor code. It makes read-only GETs, one sign-in, and always
//! signs out at the end, so the test session does not stay. It prints no secret.
//! Unattended runs (a local test server) read `ORBIT_CHECK_EMAIL` and `ORBIT_CHECK_PASSWORD_FILE`.

use std::io::Write;
use std::sync::mpsc;
use std::time::Duration;

use orbit_client::live::Ended;
use orbit_client::{
    ApiError, Client, HandOff, LiveFrame, LoginOutcome, Secret, ServerUrl, Session, SocketConfig, SocketEvent, SocketKind,
};

use crate::signin;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Forwarded,
    /// Orbit did not get the header: it answered as if there were no session.
    Stripped,
    /// The proxy refused the request because of the header.
    Rejected,
    Unknown,
}

fn verdict_http(result: &Result<impl Sized, ApiError>) -> Verdict {
    match result {
        Ok(_) => Verdict::Forwarded,
        Err(ApiError::SignedOut(_)) => Verdict::Stripped,
        Err(ApiError::GatewayRejected | ApiError::GatewaySignIn) => Verdict::Rejected,
        Err(_) => Verdict::Unknown,
    }
}

fn verdict_socket(first: &Option<SocketEvent>) -> Verdict {
    match first {
        Some(SocketEvent::Live(_) | SocketEvent::Workspace(_)) => Verdict::Forwarded,
        Some(SocketEvent::Ended(Ended::SignedOut)) => Verdict::Stripped,
        Some(SocketEvent::Ended(Ended::Gateway(_))) => Verdict::Rejected,
        _ => Verdict::Unknown,
    }
}

pub fn run(server: &str) -> i32 {
    let server = match ServerUrl::parse(server) {
        Ok(server) => server,
        Err(e) => {
            eprintln!("--check-gateway: {e}");
            return 2;
        }
    };
    println!("Risk 4b check on {server}: read-only GETs, one sign-in, sign-out at the end.\n");
    let mut client = Client::new(server.clone());

    print!("1. Probe without cookies ............ ");
    let gateway = match client.setup_status() {
        Ok(_) => { println!("200: no gateway in front of this server (4b does not apply)"); false }
        Err(ApiError::GatewaySignIn) => { println!("307 to the gateway sign-in (expected)"); true }
        Err(e) => { println!("{e}"); return 1; }
    };

    if gateway {
        println!("2. Sign-in window ................... opening; sign in to the gateway there.");
        match signin::open(&server) {
            Ok(HandOff::SignedIn(cookies)) => {
                println!("   got {} gateway cookie(s) from the window", cookies.cookies.len());
                client.set_gateway(Some(cookies));
            }
            Ok(HandOff::Cancelled) => { println!("   the window was closed: stopped"); return 1; }
            Ok(HandOff::Failed(e)) | Err(e) => { println!("   failed: {e}"); return 1; }
        }
        print!("3. Probe with the cookies ........... ");
        match client.setup_status() {
            Ok(_) => println!("200 from Orbit (the cookies pass the gateway)"),
            Err(e) => { println!("{e}: the cookies do not pass; stopped"); return 1; }
        }
        print!("4. Cookies + a junk bearer, /auth/me  ");
        client.set_session(Some(Session { session_id: String::new(), token: Secret::new("not-a-session") }));
        let junk = client.me();
        client.set_session(None);
        println!("{}", match &junk {
            Err(ApiError::GatewayRejected) => "the proxy answered 401 itself: it reads Authorization as its own token".to_owned(),
            Err(ApiError::SignedOut(_)) => "Orbit answered 401 (problem+json): the request reached Orbit".to_owned(),
            other => format!("{other:?}"),
        });
    }

    let (email, password) = match credentials() {
        Some(c) => c,
        None => { println!("No credentials: stopped before the sign-in."); return 1; }
    };
    print!("5. Orbit sign-in (no Origin) ........ ");
    let _ = std::io::stdout().flush();
    let mut outcome = client.login(&email, &password);
    if let Ok(LoginOutcome::TwoFactor(token)) = &outcome {
        println!("two-factor is on");
        let code = prompt("   Two-factor code: ");
        outcome = client.login_two_factor(token, &code);
        print!("   ");
    }
    match &outcome {
        Ok(LoginOutcome::SignedIn { .. }) => println!("signed in; session token from the body"),
        Ok(LoginOutcome::TwoFactor(_)) => { println!("two-factor again: stopped"); return 1; }
        Err(ApiError::Problem(403, p)) if p.code == "origin_forbidden" => {
            println!("403 origin_forbidden: no session was made.");
            println!("\nRisk 4b: COOKIE FORWARDED: the proxy passes its own cookie on to Orbit, and Orbit refuses a request");
            println!("without Origin that carries cookies (crates/platform/src/http/security.rs). Stop here (see apps/desktop/README.md).");
            return 1;
        }
        Err(e) => { println!("{e}: stopped"); return 1; }
    }

    let http = client.me();
    let http_verdict = verdict_http(&http);
    println!("6. /auth/me with the bearer ......... {}", describe(http_verdict, &http.as_ref().err().map(ToString::to_string)));

    let mut socket_verdicts = Vec::new();
    match client.workspaces().map(|w| w.into_iter().next()) {
        Ok(Some(workspace)) => {
            for (n, kind) in [(7, SocketKind::Live), (8, SocketKind::Events)] {
                let first = first_socket_answer(&client, &server, &workspace.id, kind);
                let verdict = verdict_socket(&first);
                println!("{n}. {:<33} {}", format!("{kind:?} socket upgrade"), describe(verdict, &first.as_ref().map(|e| format!("{e:?}"))));
                socket_verdicts.push(verdict);
            }
        }
        Ok(None) => println!("7. Sockets ........................ skipped: the account has no workspace"),
        Err(e) => println!("7. Sockets ........................ skipped: /workspaces failed: {e}"),
    }

    print!("9. Sign-out ......................... ");
    // Through a proxy that drops the bearer, a bearer sign-out would "succeed" with nothing revoked.
    let signed_out = if http_verdict == Verdict::Forwarded { client.logout() } else { client.logout_as_browser() };
    match signed_out {
        Ok(()) if http_verdict == Verdict::Forwarded => println!("done; the test session is revoked"),
        Ok(()) => println!("done with Orbit's session cookie (the bearer does not reach Orbit); the test session is revoked"),
        Err(e) => println!("FAILED ({e}): revoke the test session in the web app, Account → Sessions"),
    }

    let all: Vec<Verdict> = std::iter::once(http_verdict).chain(socket_verdicts).collect();
    let answer = if all.iter().all(|v| *v == Verdict::Forwarded) {
        "FORWARDED: the proxy passes Authorization: Bearer to Orbit on HTTP and socket upgrades. The planned auth works."
    } else if all.contains(&Verdict::Stripped) {
        "STRIPPED: Orbit did not get the bearer header. Stop here and choose another way (see apps/desktop/README.md)."
    } else if all.contains(&Verdict::Rejected) {
        "REJECTED: the proxy refuses requests that carry Authorization: Bearer. Stop here (see apps/desktop/README.md)."
    } else {
        "INCONCLUSIVE: see the lines above."
    };
    println!("\nRisk 4b: {answer}");
    i32::from(!all.iter().all(|v| *v == Verdict::Forwarded))
}

fn describe(verdict: Verdict, detail: &Option<String>) -> String {
    match verdict {
        Verdict::Forwarded => "Orbit got the bearer (signed in)".into(),
        Verdict::Stripped => "Orbit answered as signed out: the header did not reach it".into(),
        Verdict::Rejected => "the gateway refused it".into(),
        Verdict::Unknown => format!("unclear: {}", detail.as_deref().unwrap_or("no answer in 15 s")),
    }
}

/// The socket's first frame, or why it ended, within 15 seconds.
fn first_socket_answer(client: &Client, server: &ServerUrl, workspace: &str, kind: SocketKind) -> Option<SocketEvent> {
    let (tx, rx) = mpsc::channel();
    let config = SocketConfig { base_delay: Duration::from_secs(30), ..SocketConfig::default() };
    let socket = orbit_client::socket::spawn(kind, server.clone(), workspace, client.auth_headers("/api/v1/workspaces"), config, move |e| {
        let _ = tx.send(e);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut answer = None;
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(e @ (SocketEvent::Live(_) | SocketEvent::Workspace(_) | SocketEvent::Ended(_))) => {
                if !matches!(e, SocketEvent::Live(LiveFrame::Ping)) {
                    answer = Some(e);
                    break;
                }
            }
            Ok(SocketEvent::Retrying { close_code, .. }) => {
                answer = Some(SocketEvent::Retrying { after: Duration::ZERO, close_code });
                break;
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    socket.stop();
    answer
}

fn credentials() -> Option<(String, Secret)> {
    if let (Ok(email), Ok(path)) = (std::env::var("ORBIT_CHECK_EMAIL"), std::env::var("ORBIT_CHECK_PASSWORD_FILE")) {
        let password = std::fs::read_to_string(path).ok()?;
        return Some((email, Secret::new(password.trim_end_matches(['\r', '\n']))));
    }
    let email = prompt("Orbit email: ");
    let password = rpassword::prompt_password("Orbit password (not shown): ").ok()?;
    (!email.is_empty() && !password.is_empty()).then(|| (email, Secret::new(password)))
}

fn prompt(question: &str) -> String {
    print!("{question}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_client::Problem;

    #[test]
    fn verdicts_follow_who_answered() {
        assert_eq!(verdict_http(&Ok::<(), ApiError>(())), Verdict::Forwarded);
        assert_eq!(verdict_http(&Err::<(), _>(ApiError::SignedOut(Problem::default()))), Verdict::Stripped);
        assert_eq!(verdict_http(&Err::<(), _>(ApiError::GatewayRejected)), Verdict::Rejected);
        assert_eq!(verdict_socket(&Some(SocketEvent::Live(LiveFrame::Resync))), Verdict::Forwarded);
        assert_eq!(verdict_socket(&Some(SocketEvent::Ended(Ended::SignedOut))), Verdict::Stripped);
        assert_eq!(verdict_socket(&Some(SocketEvent::Ended(Ended::Gateway(ApiError::GatewayRejected)))), Verdict::Rejected);
        assert_eq!(verdict_socket(&None), Verdict::Unknown);
    }
}
