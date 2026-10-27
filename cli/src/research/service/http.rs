//! The workspace's loopback HTTP server: JSON in and out, for local
//! applications only (the host and any origin must be this loopback port).
//! `/v1/owner/restart` drains it: new requests are refused, those in
//! progress finish, then it stops and its port is free for the next one.

use super::app::{App, MAX_SUBMISSION_BODY, Request};
use crate::research::workspace::{Failure, fail_status};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const HEADER_LIMIT: usize = 16 * 1024;
const ORDINARY_BODY: usize = 32768;
const HEADERS_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
struct State {
    active: usize,
    /// No longer accepting.
    stopped: bool,
    /// Its port released and its requests finished.
    closed: bool,
}

/// A running server.
pub struct Server {
    pub base_url: String,
    port: u16,
    app: Arc<App>,
    draining: AtomicBool,
    state: Mutex<State>,
    changed: Condvar,
    accepting: Mutex<Option<std::thread::JoinHandle<()>>>,
}

/// Starts the server for `app` on 127.0.0.1:`port` (0: any free port).
pub fn start(app: Arc<App>, port: u16) -> Result<Arc<Server>, Failure> {
    let listener = TcpListener::bind(("127.0.0.1", port)).map_err(|error| Failure {
        code: Some("WORKSPACE_PORT_BUSY".into()),
        message: format!("The research workspace could not listen on port {port}: {error}"),
        status: None,
    })?;
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let server = Arc::new(Server {
        base_url: format!("http://127.0.0.1:{port}"),
        port,
        app: Arc::clone(&app),
        draining: AtomicBool::new(false),
        state: Mutex::default(),
        changed: Condvar::new(),
        accepting: Mutex::default(),
    });
    let weak = Arc::downgrade(&server);
    app.on_restart(move || {
        let Some(server) = weak.upgrade() else {
            return Ok(());
        };
        if server.lock().active > 1 {
            return Err(fail_status(
                "SERVICE_BUSY",
                "Research requests are still in progress. Retry shortly.",
                409,
            ));
        }
        server.draining.store(true, Ordering::SeqCst);
        // Let the accepted restart response finish; the close drains the rest.
        let closing = Arc::clone(&server);
        std::thread::spawn(move || closing.close());
        Ok(())
    });
    let accepting = Arc::clone(&server);
    let thread = std::thread::spawn(move || {
        for stream in listener.incoming() {
            if accepting.lock().stopped {
                break;
            }
            let Ok(stream) = stream else { continue };
            let server = Arc::clone(&accepting);
            std::thread::spawn(move || server.connection(stream));
        }
    });
    *server.accepting.lock().unwrap_or_else(|p| p.into_inner()) = Some(thread);
    app.start_discovery();
    Ok(server)
}

fn respond(stream: &mut TcpStream, status: u16, body: &Value) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    };
    let text = body.to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
    let _ = stream.flush();
}

fn refusal(error: &Failure) -> (u16, Value) {
    match error.status {
        Some(status) => (
            status,
            json!({ "ok": false, "error": { "code": error.code.as_deref().unwrap_or("INTERNAL_ERROR"), "message": error.message } }),
        ),
        None => (
            500,
            json!({ "ok": false, "error": { "code": "INTERNAL_ERROR", "message": "The workspace could not complete this request." } }),
        ),
    }
}

struct Head {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// The request line and headers; None for a malformed or oversized head.
fn read_head(reader: &mut impl BufRead) -> Result<Option<Head>, ()> {
    let mut total = 0;
    let mut lines = Vec::new();
    loop {
        let mut line = Vec::new();
        let read = reader
            .by_ref()
            .take((HEADER_LIMIT - total + 1) as u64)
            .read_until(b'\n', &mut line)
            .map_err(|_| ())?;
        if read == 0 {
            return Err(());
        }
        total += read;
        if total > HEADER_LIMIT {
            return Ok(None);
        }
        let line = String::from_utf8_lossy(&line)
            .trim_end_matches(['\r', '\n'])
            .to_string();
        if line.is_empty() {
            if lines.is_empty() {
                continue;
            }
            break;
        }
        lines.push(line);
    }
    let mut parts = lines[0].split(' ');
    let (Some(method), Some(target), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return Ok(None);
    };
    if !version.starts_with("HTTP/1.") {
        return Ok(None);
    }
    let mut headers = Vec::new();
    for line in &lines[1..] {
        let Some((name, value)) = line.split_once(':') else {
            return Ok(None);
        };
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Ok(Some(Head {
        method: method.into(),
        target: target.into(),
        headers,
    }))
}

/// A chunked body, at most `limit` bytes (None: over it or malformed).
fn read_chunked(reader: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>, ()> {
    let mut body = Vec::new();
    loop {
        let mut line = String::new();
        reader
            .by_ref()
            .take(1024)
            .read_line(&mut line)
            .map_err(|_| ())?;
        let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| ())?;
        if size == 0 {
            // Trailers, up to the blank line.
            loop {
                let mut trailer = String::new();
                if reader
                    .by_ref()
                    .take(1024)
                    .read_line(&mut trailer)
                    .map_err(|_| ())?
                    == 0
                    || trailer.trim().is_empty()
                {
                    return Ok(Some(body));
                }
            }
        }
        if body.len() + size > limit {
            return Ok(None);
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..]).map_err(|_| ())?;
        let mut end = [0; 2];
        reader.read_exact(&mut end).map_err(|_| ())?;
    }
}

impl Server {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Stops accepting and releases the port, lets the requests in progress
    /// finish, then closes the application.
    pub fn close(&self) {
        self.draining.store(true, Ordering::SeqCst);
        {
            let mut state = self.lock();
            if state.stopped {
                drop(state);
                return self.wait();
            }
            state.stopped = true;
        }
        // Wake the accepting thread so it lets go of the port.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self
            .accepting
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            let _ = thread.join();
        }
        let mut state = self.lock();
        while state.active > 0 {
            state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        drop(state);
        self.app.close();
        self.lock().closed = true;
        self.changed.notify_all();
    }

    /// Blocks until the server has closed (a restart, or `close`).
    pub fn wait(&self) {
        let mut state = self.lock();
        while !state.closed {
            state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
        }
    }

    fn connection(&self, mut stream: TcpStream) {
        let started = Instant::now();
        let _ = stream.set_read_timeout(Some(HEADERS_TIMEOUT));
        let Ok(reader) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(reader);
        let head = match read_head(&mut reader) {
            Ok(Some(head)) => head,
            Ok(None) => {
                respond(
                    &mut stream,
                    431,
                    &json!({ "ok": false, "error": { "code": "INVALID_REQUEST", "message": "Invalid request." } }),
                );
                return;
            }
            Err(()) => return,
        };
        self.lock().active += 1;
        // The request's slot is released however it ends.
        struct Active<'a>(&'a Server);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                self.0.lock().active -= 1;
                self.0.changed.notify_all();
            }
        }
        let _active = Active(self);
        let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.request(&head, &mut reader, started)
        }));
        let (status, body) = match handled {
            Ok(Ok(data)) => (200, json!({ "ok": true, "data": data })),
            Ok(Err(error)) => refusal(&error),
            Err(_) => refusal(&Failure {
                code: None,
                message: "The request failed.".into(),
                status: None,
            }),
        };
        respond(&mut stream, status, &body);
        let _ = stream.shutdown(Shutdown::Write);
    }

    fn request(
        &self,
        head: &Head,
        reader: &mut BufReader<TcpStream>,
        started: Instant,
    ) -> Result<Value, Failure> {
        if self.draining.load(Ordering::SeqCst) {
            return Err(fail_status(
                "WORKSPACE_UNAVAILABLE",
                "Petri is restarting. Retry shortly.",
                503,
            ));
        }
        let hosts = [
            format!("127.0.0.1:{}", self.port),
            format!("localhost:{}", self.port),
        ];
        let host_ok = head
            .header("host")
            .is_some_and(|host| hosts.iter().any(|h| h == host));
        let origin_ok = head
            .header("origin")
            .is_none_or(|origin| hosts.iter().any(|h| origin == format!("http://{h}")));
        if !host_ok || !origin_ok {
            return Err(fail_status(
                "ORIGIN_DENIED",
                "This workspace accepts local application requests only.",
                403,
            ));
        }
        if head.method != "GET" && head.method != "POST" {
            return Err(fail_status(
                "METHOD_NOT_ALLOWED",
                "Method not allowed.",
                405,
            ));
        }
        let invalid_path = || fail_status("INVALID_PATH", "Invalid request path.", 400);
        let url = reqwest::Url::parse("http://127.0.0.1/")
            .and_then(|base| base.join(&head.target))
            .map_err(|_| invalid_path())?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid_path());
        }
        let path = url.path().to_string();
        let query: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let (body, raw) = if head.method == "POST" {
            let json_type = head.header("content-type").is_some_and(|t| {
                let t = t.to_ascii_lowercase();
                t.strip_prefix("application/json")
                    .is_some_and(|rest| rest.is_empty() || rest.trim_start().starts_with(';'))
            });
            if !json_type {
                return Err(fail_status(
                    "INVALID_CONTENT_TYPE",
                    "Use application/json.",
                    415,
                ));
            }
            // A candidate may bring a web proof bundle.
            let candidates = path.strip_prefix("/v1/runs/").is_some_and(|rest| {
                rest.len() == 36 + "/candidates".len()
                    && rest.ends_with("/candidates")
                    && rest[..36]
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
            });
            let limit = if candidates {
                MAX_SUBMISSION_BODY
            } else {
                ORDINARY_BODY
            };
            let too_large = || fail_status("BODY_TOO_LARGE", "The request is too large.", 413);
            let remaining = REQUEST_TIMEOUT
                .saturating_sub(started.elapsed())
                .max(Duration::from_millis(1));
            let _ = reader.get_ref().set_read_timeout(Some(remaining));
            let bytes = if head
                .header("transfer-encoding")
                .is_some_and(|t| t.eq_ignore_ascii_case("chunked"))
            {
                read_chunked(reader, limit)
                    .map_err(|_| fail_status("INVALID_JSON", "Invalid JSON request.", 400))?
                    .ok_or_else(too_large)?
            } else {
                let length: usize = head
                    .header("content-length")
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| fail_status("INVALID_JSON", "Invalid JSON request.", 400))?;
                if length > limit {
                    // Read what was sent, so the client sees this answer.
                    if length <= 8 * 1024 * 1024 {
                        let _ = std::io::copy(
                            &mut reader.by_ref().take(length as u64),
                            &mut std::io::sink(),
                        );
                    }
                    return Err(too_large());
                }
                let mut bytes = vec![0; length];
                reader
                    .read_exact(&mut bytes)
                    .map_err(|_| fail_status("INVALID_JSON", "Invalid JSON request.", 400))?;
                bytes
            };
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let body: Value = serde_json::from_str(&text)
                .map_err(|_| fail_status("INVALID_JSON", "Invalid JSON request.", 400))?;
            (body, Some(text))
        } else {
            (json!({}), None)
        };
        let credential = head.header("authorization").and_then(|value| {
            let token = value.strip_prefix("Bearer ")?;
            (token.len() == 64
                && token
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            .then(|| token.to_string())
        });
        self.app.handle(&Request {
            method: head.method.clone(),
            path,
            query,
            credential,
            body,
            raw,
        })
    }
}
