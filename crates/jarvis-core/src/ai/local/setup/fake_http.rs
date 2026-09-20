//! A tiny HTTP/1.1 server for the setup tests.
//!
//! It exists so the download logic can be exercised through the *real*
//! `reqwest` transport against a real socket, with responses that are awkward on
//! purpose: a missing `Content-Length`, a `200` where a `206` was asked for, a
//! body that stops halfway, a refused range. No test in this repository ever
//! downloads a real artifact.
//!
//! It is only compiled for tests, and it binds `127.0.0.1` on a port the
//! operating system chooses.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// One request the server received.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    /// First byte the client asked for, when it sent a `Range` header.
    pub range_start: Option<u64>,
    /// Whether the client asked for a range at all.
    pub had_range: bool,
}

/// How one request is answered.
#[derive(Clone, Debug)]
pub enum FakeResponse {
    /// `200` with a `Content-Length` and an `ETag`, and the whole body.
    Full { body: Vec<u8>, etag: Option<String> },
    /// `200` with a `Content-Length` that is deliberately not the body length.
    FullWithLength { body: Vec<u8>, length: u64, etag: Option<String> },
    /// `200` without `Content-Length`; the body ends when the socket closes.
    CloseDelimited { body: Vec<u8> },
    /// `200` that advertises the whole body but sends only `prefix` bytes.
    Truncated { body: Vec<u8>, prefix: usize, etag: Option<String> },
    /// `206` for the requested range, with a matching `Content-Range`.
    Ranged { body: Vec<u8>, etag: Option<String> },
    /// `206` that claims a range starting somewhere the client did not ask for.
    RangedFrom { body: Vec<u8>, start: u64, etag: Option<String> },
    /// `200` regardless of the `Range` header.
    IgnoreRange { body: Vec<u8>, etag: Option<String> },
    /// A status with no body.
    Status { code: u16 },
    /// `200` written in pieces, so a test can cancel in the middle.
    Throttled { body: Vec<u8>, chunk: usize },
}

impl FakeResponse {
    const DEFAULT_ETAG: &'static str = "\"v1\"";

    /// A complete, ranged-capable response.
    pub fn full(body: Vec<u8>) -> Self {
        Self::Full {
            body,
            etag: Some(Self::DEFAULT_ETAG.to_string()),
        }
    }

    /// The same as [`Self::full`], with a `Content-Length` that lies.
    pub fn full_with_length(body: Vec<u8>, length: u64) -> Self {
        Self::FullWithLength {
            body,
            length,
            etag: Some(Self::DEFAULT_ETAG.to_string()),
        }
    }

    /// `200` with an honest `Content-Length`.
    pub fn exact(body: Vec<u8>) -> Self {
        Self::full(body)
    }

    /// `200` with no `Content-Length` at all.
    pub fn close_delimited(body: Vec<u8>) -> Self {
        Self::CloseDelimited { body }
    }

    /// `200` that stops after `prefix` bytes.
    pub fn truncated(body: Vec<u8>, prefix: usize) -> Self {
        Self::Truncated {
            body,
            prefix,
            etag: Some(Self::DEFAULT_ETAG.to_string()),
        }
    }

    /// `206` for whatever range the client asked for.
    pub fn ranged(body: Vec<u8>) -> Self {
        Self::Ranged {
            body,
            etag: Some(Self::DEFAULT_ETAG.to_string()),
        }
    }

    /// `206` for a range, with a chosen `ETag`.
    pub fn ranged_with_etag(body: Vec<u8>, etag: &str) -> Self {
        Self::Ranged {
            body,
            etag: Some(etag.to_string()),
        }
    }

    /// `206` that describes a different range than the one requested.
    pub fn ranged_from(body: Vec<u8>, start: u64, etag: &str) -> Self {
        Self::RangedFrom {
            body,
            start,
            etag: Some(etag.to_string()),
        }
    }

    /// `200` for every request, even a ranged one.
    pub fn ignore_range(body: Vec<u8>) -> Self {
        Self::IgnoreRange {
            body,
            etag: Some(Self::DEFAULT_ETAG.to_string()),
        }
    }

    /// A status with no body.
    pub fn status(code: u16) -> Self {
        Self::Status { code }
    }

    /// A body written in pieces, so a test can cancel in the middle.
    pub fn throttled(body: Vec<u8>, chunk: usize) -> Self {
        Self::Throttled { body, chunk }
    }
}

struct ServerState {
    script: Vec<FakeResponse>,
    requests: Vec<RecordedRequest>,
}

/// A running fake server. Dropping it stops the accept loop.
pub struct FakeServer {
    address: std::net::SocketAddr,
    state: Arc<Mutex<ServerState>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeServer {
    /// Starts a server on a loopback port chosen by the operating system.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let address = listener.local_addr().expect("the bound address");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        let state = Arc::new(Mutex::new(ServerState {
            script: Vec::new(),
            requests: Vec::new(),
        }));
        let running = Arc::new(AtomicBool::new(true));
        let handle = {
            let state = Arc::clone(&state);
            let running = Arc::clone(&running);
            thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = stream.set_nodelay(true);
                            // On Windows an accepted socket inherits the
                            // listener's non-blocking mode, and a read on it then
                            // fails immediately with `WSAEWOULDBLOCK` instead of
                            // waiting for the request. Every connection here is
                            // handled synchronously, so the stream must block.
                            let _ = stream.set_nonblocking(false);
                            serve(stream, &state);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(_) => {
                            // A connection that was reset while it sat in the
                            // backlog is reported here. It is not a reason to
                            // stop serving: the next request still deserves an
                            // answer, and a test that silently lost its server
                            // would be far harder to diagnose.
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                }
            })
        };
        Self {
            address,
            state,
            running,
            handle: Some(handle),
        }
    }

    /// The URL this server answers on, for a given path.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}/{}", self.address, path)
    }

    /// Queues the next response.
    pub fn push(&self, response: FakeResponse) {
        self.state.lock().unwrap().script.push(response);
    }

    /// Every request the server has received, in order.
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        // Nudge the accept loop out of its sleep and close the socket.
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve(mut stream: TcpStream, state: &Arc<Mutex<ServerState>>) {
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let response = {
        let mut state = state.lock().unwrap();
        state.requests.push(request.clone());
        if state.script.is_empty() {
            FakeResponse::Status { code: 500 }
        } else {
            state.script.remove(0)
        }
    };
    let _ = write_response(&mut stream, &request, &response);
    let _ = stream.flush();
    // `Write` rather than `Both`: the response is already queued, and closing
    // the read side of a socket with data still queued can turn a clean
    // end-of-body into a reset on Windows.
    let _ = stream.shutdown(Shutdown::Write);
}

/// Reads one request head. The body is never needed: every request is a `GET`.
fn read_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let count = match stream.read(&mut chunk) {
            Ok(count) => count,
            Err(_) => return None,
        };
        if count == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..count]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if buffer.len() > 64 * 1024 {
            return None;
        }
    }
    let text = String::from_utf8_lossy(&buffer).into_owned();
    let mut lines = text.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut range_start = None;
    let mut had_range = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("range") {
            had_range = true;
            let value = value.trim();
            if let Some(rest) = value.strip_prefix("bytes=") {
                let start = rest.split('-').next().unwrap_or_default();
                range_start = start.trim().parse::<u64>().ok();
            }
        }
    }
    Some(RecordedRequest {
        method,
        path,
        range_start,
        had_range,
    })
}

fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        206 => "Partial Content",
        416 => "Range Not Satisfiable",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

fn write_head(
    stream: &mut TcpStream,
    code: u16,
    headers: &[(&str, String)],
) -> std::io::Result<()> {
    let mut head = format!("HTTP/1.1 {code} {}\r\n", reason(code));
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    stream.write_all(head.as_bytes())
}

fn write_response(
    stream: &mut TcpStream,
    request: &RecordedRequest,
    response: &FakeResponse,
) -> std::io::Result<()> {
    let etag = |etag: &Option<String>| etag.clone().unwrap_or_else(|| "\"v1\"".to_string());
    match response {
        FakeResponse::Full { body, etag: tag } => {
            write_head(
                stream,
                200,
                &[
                    ("Content-Length", body.len().to_string()),
                    ("Content-Type", "application/octet-stream".to_string()),
                    ("Accept-Ranges", "bytes".to_string()),
                    ("ETag", etag(tag)),
                ],
            )?;
            stream.write_all(body)
        }
        FakeResponse::FullWithLength {
            body,
            length,
            etag: tag,
        } => {
            write_head(
                stream,
                200,
                &[
                    ("Content-Length", length.to_string()),
                    ("Accept-Ranges", "bytes".to_string()),
                    ("ETag", etag(tag)),
                ],
            )?;
            stream.write_all(body)
        }
        FakeResponse::CloseDelimited { body } => {
            write_head(stream, 200, &[("Content-Type", "application/octet-stream".to_string())])?;
            stream.write_all(body)
        }
        FakeResponse::Truncated {
            body,
            prefix,
            etag: tag,
        } => {
            write_head(
                stream,
                200,
                &[
                    ("Content-Length", body.len().to_string()),
                    ("Accept-Ranges", "bytes".to_string()),
                    ("ETag", etag(tag)),
                ],
            )?;
            let take = (*prefix).min(body.len());
            stream.write_all(&body[..take])?;
            stream.flush()
        }
        FakeResponse::Ranged { body, etag: tag } => {
            let start = request.range_start.unwrap_or(0);
            write_range(stream, body, start, tag)
        }
        FakeResponse::RangedFrom {
            body,
            start,
            etag: tag,
        } => write_range(stream, body, *start, tag),
        FakeResponse::IgnoreRange { body, etag: tag } => {
            write_head(
                stream,
                200,
                &[
                    ("Content-Length", body.len().to_string()),
                    ("Accept-Ranges", "bytes".to_string()),
                    ("ETag", etag(tag)),
                ],
            )?;
            stream.write_all(body)
        }
        FakeResponse::Status { code } => write_head(stream, *code, &[("Content-Length", "0".to_string())]),
        FakeResponse::Throttled { body, chunk } => {
            write_head(
                stream,
                200,
                &[
                    ("Content-Length", body.len().to_string()),
                    ("Accept-Ranges", "bytes".to_string()),
                ],
            )?;
            let size = (*chunk).max(1);
            for piece in body.chunks(size) {
                if stream.write_all(piece).is_err() {
                    // The client cancelled and closed the socket.
                    return Ok(());
                }
                let _ = stream.flush();
            }
            Ok(())
        }
    }
}

fn write_range(
    stream: &mut TcpStream,
    body: &[u8],
    start: u64,
    tag: &Option<String>,
) -> std::io::Result<()> {
    let start = start.min(body.len() as u64);
    let tail = &body[start as usize..];
    let end = (body.len() as u64).saturating_sub(1);
    write_head(
        stream,
        206,
        &[
            ("Content-Length", tail.len().to_string()),
            ("Content-Range", format!("bytes {start}-{end}/{}", body.len())),
            ("Accept-Ranges", "bytes".to_string()),
            ("ETag", tag.clone().unwrap_or_else(|| "\"v1\"".to_string())),
        ],
    )?;
    stream.write_all(tail)
}


