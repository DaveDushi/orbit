//! A scripted local server for the client tests: each accepted connection runs the next step.
//! Synthetic data only; it listens on 127.0.0.1 at a free port.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tungstenite::WebSocket;
use tungstenite::handshake::server::{Request, Response};

/// One request as the server saw it: the request line, the headers (lowercase names) and the body.
#[derive(Clone, Debug, Default)]
pub struct Seen {
    pub line: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

pub type Step = Box<dyn FnOnce(TcpStream, &Mutex<Vec<Seen>>) + Send>;

pub struct Mock {
    pub origin: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
    thread: Option<JoinHandle<()>>,
}

impl Mock {
    pub fn start(steps: Vec<Step>) -> Mock {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let thread = std::thread::spawn({
            let seen = seen.clone();
            move || {
                for step in steps {
                    let (stream, _) = listener.accept().unwrap();
                    step(stream, &seen);
                }
            }
        });
        Mock { origin, seen, thread: Some(thread) }
    }

    /// Waits until every step ran, then gives what the server saw.
    pub fn finish(mut self) -> Vec<Seen> {
        self.thread.take().unwrap().join().unwrap();
        self.seen.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> Seen {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut seen = Seen { line: line.trim_end().to_owned(), ..Seen::default() };
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        seen.headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
    }
    let length: usize = seen.header("content-length").map_or(0, |v| v.parse().unwrap());
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    seen.body = String::from_utf8(body).unwrap();
    seen
}

/// Answers one HTTP request with `status`, extra `headers` and `body`.
pub fn http(status: u16, headers: &'static [(&'static str, &'static str)], body: &'static str) -> Step {
    Box::new(move |mut stream, seen| {
        seen.lock().unwrap().push(read_request(&mut stream));
        let mut response = format!("HTTP/1.1 {status} X\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
        for (name, value) in headers {
            response.push_str(&format!("{name}: {value}\r\n"));
        }
        response.push_str("\r\n");
        response.push_str(body);
        stream.write_all(response.as_bytes()).unwrap();
    })
}

/// Accepts a WebSocket upgrade and hands the socket to `script`.
#[allow(clippy::result_large_err)] // tungstenite's handshake callback type
pub fn ws(script: impl FnOnce(&mut WebSocket<TcpStream>) + Send + 'static) -> Step {
    Box::new(move |stream, seen| {
        let mut request_seen = Seen::default();
        let mut socket = tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
            request_seen.line = format!("{} {}", request.method(), request.uri());
            for (name, value) in request.headers() {
                request_seen.headers.push((name.as_str().to_owned(), value.to_str().unwrap().to_owned()));
            }
            Ok(response)
        })
        .unwrap();
        seen.lock().unwrap().push(request_seen);
        script(&mut socket);
        // Let the client read the close before the socket goes.
        let _ = socket.flush();
        while socket.read().is_ok() {}
    })
}
