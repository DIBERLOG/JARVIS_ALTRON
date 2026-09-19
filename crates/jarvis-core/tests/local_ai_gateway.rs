//! Integration tests for the local AI layer: the loopback HTTP/SSE client, the
//! GGUF and configuration checks, and the gateway lifecycle.
//!
//! What this file proves, over real TCP sockets and real files:
//!
//! * the client reads `Content-Length`, `chunked`, and close-delimited bodies
//!   correctly, including lines and UTF-8 characters split across TCP writes;
//! * a full SSE completion (`data:` fragments, a comment keep-alive, an event
//!   with no data field, `[DONE]`, and a final usage/finish chunk) becomes typed
//!   outcomes, and every failure shape (4xx, 5xx, malformed chunk, truncation,
//!   stall, oversized body) becomes a controlled `ChatError`;
//! * a cancellation flag set by another thread ends a stalled stream promptly;
//! * a synthetic GGUF file is accepted while non-GGUF, missing, and misnamed
//!   files are refused, hostile model paths stay one argument, and the shape
//!   validation names every refused field;
//! * the request that actually crosses the socket carries no credential field or
//!   header;
//! * the gateway reaches `Ready`, streams into an `EventSink`, reports a
//!   cancellation, and returns to `Stopped`, using only a fake process runner.
//!
//! Nothing here starts `llama-server`, opens a non-loopback socket, or downloads
//! anything. Every mock socket has a timeout and every asynchronous assertion
//! has a deadline, so the binary cannot hang.

use std::io::{self, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use jarvis_core::ai::local::client::{cancellation_flag, request_cancel};
use jarvis_core::ai::local::{
    loopback_address, validate_files, validate_model, BodyReader, ChatCompletionRequest,
    CheckLevel, ChunkOutcome, CompletionOptions, EventSink, GenerationEvent, GenerationRequest,
    GenerationUsage, HealthState, LocalAiClient, LocalAiConfig, LocalAiGateway, LocalAiState,
    LocalModelConfig, LoopbackEndpoint, ProcessRunner, ServerExit, ServerProcess, SpawnedServer,
    ThinkingMode, CHAT_COMPLETIONS_PATH, HEALTH_PATH, MAX_BODY_BYTES, MODELS_PATH, PROPS_PATH,
};
use jarvis_core::ai::{ChatError, ChatMessage, ChatRole, Persona};
use tempfile::tempdir;

/// Every mock socket carries a timeout, so a bug in a fixture fails a test
/// instead of hanging the whole test binary.
const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

/// Deadline for waiting on an effect produced by another thread.
const EVENT_DEADLINE: Duration = Duration::from_secs(10);

/// Polling interval while waiting for an effect produced by another thread.
const EVENT_POLL: Duration = Duration::from_millis(10);

/// Model list of a server that reports a usable identifier.
const MODELS_WITH_ID: &str =
    r#"{"object":"list","data":[{"id":"mock-qwen3-8b","object":"model"}]}"#;

/// Model list of a server that reports no usable identifier.
const MODELS_WITHOUT_ID: &str = r#"{"object":"list","data":[{"id":"","object":"model"}]}"#;

/// A `/props` payload whose template mentions the thinking switch this client
/// looks for before it sends a preference.
const TEMPLATE_WITH_THINKING: &str =
    r#"{"chat_template":"{% if enable_thinking %}think{% endif %}","build_info":"b9999-test"}"#;

/// A `/props` payload without a `chat_template` field at all.
const TEMPLATE_ABSENT: &str = r#"{"build_info":"b1-test"}"#;

// ---------------------------------------------------------------------------
// A mock loopback HTTP server
// ---------------------------------------------------------------------------

/// One request exactly as the mock server received it.
#[derive(Clone, Debug)]
struct MockRequest {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl MockRequest {
    /// Header lookup, case-insensitively, as HTTP requires.
    fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The body parsed as JSON, so assertions read fields instead of substrings.
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("the client must send a JSON request body")
    }
}

/// How the mock server answers one request: it gets the parsed request and a
/// writable socket, and decides the whole response itself.
type Responder = Arc<dyn Fn(&MockRequest, &mut TcpStream) + Send + Sync>;

/// A loopback HTTP server with a non-blocking accept loop and one thread per
/// connection.
///
/// `Drop` stops the listener. Connections already accepted finish on their own
/// because their sockets have timeouts, which is what lets a test exercise a
/// stall without leaking a blocked thread into the next test.
struct MockServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
}

impl MockServer {
    /// Binds port 0 on `127.0.0.1` and answers every request with `responder`.
    fn start<F>(responder: F) -> Self
    where
        F: Fn(&MockRequest, &mut TcpStream) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("a loopback port must be free");
        listener
            .set_nonblocking(true)
            .expect("the mock listener must become non-blocking");
        let address = listener
            .local_addr()
            .expect("the mock listener must have an address");
        let stop = Arc::new(AtomicBool::new(false));
        let responder: Responder = Arc::new(responder);

        let accept_stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !accept_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let responder = Arc::clone(&responder);
                        // One thread per connection: a deliberately stalled
                        // response must not block the next request.
                        std::thread::spawn(move || handle_connection(stream, &responder));
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    // The listener is gone: there is nothing left to serve.
                    Err(_) => break,
                }
            }
        });

        Self { address, stop }
    }

    /// The port the operating system assigned.
    fn port(&self) -> u16 {
        self.address.port()
    }

    /// A production client pointed at this mock server.
    fn client(&self) -> LocalAiClient {
        LocalAiClient::new("127.0.0.1", self.port()).expect("loopback is always accepted")
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Serves one accepted connection; every failure ends the connection quietly
/// because a test assertion, not this thread, decides the outcome.
fn handle_connection(mut stream: TcpStream, responder: &Responder) {
    // The listener is non-blocking, and on Windows the accepted socket inherits
    // that mode. Without putting it back, a large body is cut short by an
    // immediate `WouldBlock` instead of waiting for the client to read, and a
    // request that has not arrived yet looks like a closed connection.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(SOCKET_TIMEOUT));
    let _ = stream.set_write_timeout(Some(SOCKET_TIMEOUT));
    let _ = stream.set_nodelay(true);
    if let Some(request) = read_request(&mut stream) {
        responder(&request, &mut stream);
    }
    let _ = stream.shutdown(Shutdown::Both);
}

/// Reads one request from a mock connection.
///
/// The head is read byte by byte so no body byte is swallowed, and the body is
/// read by `Content-Length`, which is the only framing this client sends.
fn read_request(stream: &mut TcpStream) -> Option<MockRequest> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if head.len() > 64 * 1024 {
            return None;
        }
        match stream.read(&mut byte) {
            Ok(0) => return None,
            Ok(_) => head.push(byte[0]),
            Err(_) => return None,
        }
        if head.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let head = String::from_utf8(head).ok()?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_string();
    let path = request_line.next()?.to_string();

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }

    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    if length > 0 && stream.read_exact(&mut body).is_err() {
        return None;
    }

    Some(MockRequest {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Writes a JSON response with `Content-Length`.
fn respond_json(stream: &mut TcpStream, status: u16, body: &[u8]) -> io::Result<()> {
    respond_with_length(stream, status, "application/json", body.len(), body)
}

/// Writes a response that declares `declared` body bytes and then sends `body`.
///
/// Declaring fewer bytes than are sent is how the framing test proves that the
/// declared length, not the peer's close, bounds the body.
fn respond_with_length(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    declared: usize,
    body: &[u8],
) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n",
        reason_phrase(status)
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// A short reason phrase, so the mock responses look like real ones.
fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        418 => "I'm a teapot",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

/// Sends a `text/event-stream` body with its length declared up front, then
/// writes the pieces one at a time.
///
/// Each piece is a separate write, so a line or a UTF-8 character can be split
/// across TCP segments and the client has to reassemble both.
fn respond_sse(stream: &mut TcpStream, pieces: &[Vec<u8>]) -> io::Result<()> {
    let total: usize = pieces.iter().map(Vec::len).sum();
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;
    stream.flush()?;
    write_pieces(stream, pieces)
}

/// Writes `pieces` as separate TCP writes with a short pause between them, so
/// they arrive as separate reads. TCP may still coalesce them, and the
/// assertions hold either way.
fn write_pieces(stream: &mut TcpStream, pieces: &[Vec<u8>]) -> io::Result<()> {
    for piece in pieces {
        stream.write_all(piece)?;
        stream.flush()?;
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

/// Frames `body` as `Transfer-Encoding: chunked`, deliberately using a small odd
/// chunk size so chunk boundaries fall inside lines and inside UTF-8 characters.
fn respond_chunked(stream: &mut TcpStream, content_type: &str, body: &[u8]) -> io::Result<()> {
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;

    const CHUNK: usize = 7;
    let mut offset = 0;
    while offset < body.len() {
        let end = (offset + CHUNK).min(body.len());
        stream.write_all(format!("{:x}\r\n", end - offset).as_bytes())?;
        stream.write_all(&body[offset..end])?;
        stream.write_all(b"\r\n")?;
        stream.flush()?;
        offset = end;
    }
    stream.write_all(b"0\r\n\r\n")?;
    stream.flush()
}

/// Sends a body with no framing header: the client reads until the peer closes.
fn respond_until_close(stream: &mut TcpStream, content_type: &str, body: &[u8]) -> io::Result<()> {
    let head =
        format!("HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// Answers the three probe paths, so `probe()` and `health()` can be driven
/// without a model. The health status is mutable so one server can prove all
/// three mappings.
struct ProbeScript {
    health_status: AtomicU16,
    models_body: &'static str,
    props_body: &'static str,
}

impl ProbeScript {
    fn new(health_status: u16, models_body: &'static str, props_body: &'static str) -> Arc<Self> {
        Arc::new(Self {
            health_status: AtomicU16::new(health_status),
            models_body,
            props_body,
        })
    }

    fn responder(
        self: &Arc<Self>,
    ) -> impl Fn(&MockRequest, &mut TcpStream) + Send + Sync + 'static {
        let script = Arc::clone(self);
        move |request: &MockRequest, stream: &mut TcpStream| match request.path.as_str() {
            HEALTH_PATH => {
                let status = script.health_status.load(Ordering::SeqCst);
                let _ = respond_json(stream, status, b"{\"status\":\"ok\"}");
            }
            MODELS_PATH => {
                let _ = respond_json(stream, 200, script.models_body.as_bytes());
            }
            PROPS_PATH => {
                let _ = respond_json(stream, 200, script.props_body.as_bytes());
            }
            _ => {
                let _ = respond_json(stream, 404, b"{}");
            }
        }
    }
}

/// Answers every chat request with the given SSE fragments.
fn sse_responder(
    pieces: Vec<Vec<u8>>,
) -> impl Fn(&MockRequest, &mut TcpStream) + Send + Sync + 'static {
    move |request: &MockRequest, stream: &mut TcpStream| {
        if request.path == CHAT_COMPLETIONS_PATH {
            let _ = respond_sse(stream, &pieces);
        } else {
            let _ = respond_json(stream, 404, b"{}");
        }
    }
}

/// Records every request and answers chat calls with the given fragments.
fn capturing_responder(
    captured: Arc<Mutex<Vec<MockRequest>>>,
    pieces: Vec<Vec<u8>>,
) -> impl Fn(&MockRequest, &mut TcpStream) + Send + Sync + 'static {
    move |request: &MockRequest, stream: &mut TcpStream| {
        captured.lock().unwrap().push(request.clone());
        if request.path == CHAT_COMPLETIONS_PATH {
            let _ = respond_sse(stream, &pieces);
        } else {
            let _ = respond_json(stream, 404, b"{}");
        }
    }
}

/// Two tokens, then silence with the body still open: only the cancellation flag
/// can end this stream.
fn respond_tokens_then_stall(stream: &mut TcpStream) -> io::Result<()> {
    stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n",
    )?;
    stream.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"half\"}}]}\n\n")?;
    stream.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\" \"}}]}\n\n")?;
    stream.flush()?;
    std::thread::sleep(SOCKET_TIMEOUT);
    Ok(())
}

// ---------------------------------------------------------------------------
// SSE fixtures
// ---------------------------------------------------------------------------

/// One SSE completion whose single delta carries reasoning and answer text.
fn reasoning_stream_pieces() -> Vec<Vec<u8>> {
    vec![
        b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"step by step\",\"content\":\"the answer\"}}]}\n\n".to_vec(),
        b"data: [DONE]\n\n".to_vec(),
    ]
}

/// The fragments of one complete SSE completion.
///
/// The list is written on purpose as separate TCP writes: one data line is split
/// mid-JSON between two writes, and a Cyrillic token is split inside its UTF-8
/// encoding. It also contains a comment keep-alive, an empty `data:` line, an
/// event with no data field at all, and the trailing `[DONE]` a real server sends
/// after the usage chunk.
fn complete_stream_pieces() -> Vec<Vec<u8>> {
    let cyrillic = "data: {\"choices\":[{\"delta\":{\"content\":\"привет\"}}]}\n\n";
    let bytes = cyrillic.as_bytes();
    // Stop after the first byte of the first multi-byte character.
    let split = bytes
        .iter()
        .position(|byte| *byte >= 0x80)
        .expect("the fixture contains Cyrillic")
        + 1;

    vec![
        b": keep-alive\n\n".to_vec(),
        b"data: \n\n".to_vec(),
        b"data: {\"choices\":[{\"delta\":{\"content\":\"Hel".to_vec(),
        b"lo\"}}]}\n\n".to_vec(),
        bytes[..split].to_vec(),
        bytes[split..].to_vec(),
        b"event: ping\n\n".to_vec(),
        b"data: {\"choices\":[{\"delta\":{\"content\":\"!\"}}]}\n\n".to_vec(),
        b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":4,\"total_tokens\":15}}\n\n".to_vec(),
        b"data: [DONE]\n\n".to_vec(),
    ]
}

/// The outcomes the complete fixture must produce, whatever the framing is.
fn expected_complete_outcomes() -> Vec<ChunkOutcome> {
    vec![
        ChunkOutcome::Token("Hello".to_string()),
        ChunkOutcome::Token("привет".to_string()),
        ChunkOutcome::Token("!".to_string()),
        ChunkOutcome::Completed {
            finish_reason: Some("stop".to_string()),
            usage: Some(GenerationUsage {
                prompt_tokens: 11,
                completion_tokens: 4,
                total_tokens: 15,
            }),
        },
    ]
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Builds a streaming request for the mock endpoint.
///
/// The profile is `Jarvis` and the question is fictional: no test in this file
/// uses real conversation content.
fn chat_request(stream: bool) -> ChatCompletionRequest {
    ChatCompletionRequest::build(
        "mock-qwen3-8b",
        Persona::Jarvis,
        &[ChatMessage {
            role: ChatRole::User,
            content: "FICTIONAL_USER_QUESTION".to_string(),
        }],
        CompletionOptions {
            thinking: ThinkingMode::Auto,
            max_tokens: 64,
            temperature: 0.7,
            top_p: 0.95,
            stream,
            thinking_supported: false,
        },
    )
}

/// Runs one streaming completion and returns its result with every outcome.
fn stream_outcomes(
    client: &LocalAiClient,
    request: &ChatCompletionRequest,
) -> (Result<(), ChatError>, Vec<ChunkOutcome>) {
    let cancel = cancellation_flag();
    let mut outcomes = Vec::new();
    let result = client.stream_chat(request, &cancel, &mut |outcome| outcomes.push(outcome));
    (result, outcomes)
}

/// The text of every token outcome, in order.
fn token_text(outcomes: &[ChunkOutcome]) -> String {
    outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            ChunkOutcome::Token(text) => Some(text.as_str()),
            // A chunk that carries reasoning as well still holds answer text.
            ChunkOutcome::TokenAndThinking { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// The completion outcome of a stream, if it reported one.
fn completion(outcomes: &[ChunkOutcome]) -> Option<(Option<String>, Option<GenerationUsage>)> {
    outcomes.iter().find_map(|outcome| match outcome {
        ChunkOutcome::Completed {
            finish_reason,
            usage,
        } => Some((finish_reason.clone(), *usage)),
        _ => None,
    })
}

/// Short labels of the events collected so far, for failure messages.
fn event_kinds(events: &[GenerationEvent]) -> Vec<&'static str> {
    events.iter().map(GenerationEvent::kind).collect()
}

/// A sink that appends every event, so a test can assert on the sequence.
fn event_sink(events: &Arc<Mutex<Vec<GenerationEvent>>>) -> EventSink {
    let events = Arc::clone(events);
    Arc::new(move |event| events.lock().unwrap().push(event))
}

/// Waits until `predicate` holds for the collected events, or panics after the
/// deadline: an asynchronous assertion must fail, never hang.
fn wait_for(
    events: &Arc<Mutex<Vec<GenerationEvent>>>,
    predicate: impl Fn(&[GenerationEvent]) -> bool,
) {
    let deadline = Instant::now() + EVENT_DEADLINE;
    loop {
        if predicate(&events.lock().unwrap()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for an event; collected {:?}",
            event_kinds(&events.lock().unwrap())
        );
        std::thread::sleep(EVENT_POLL);
    }
}

/// Runs `action` on its own thread and fails the test if it does not settle.
///
/// The gateway's state lock is not reentrant, so a regression there would block
/// the calling thread forever and take the whole test binary (and cargo) with it.
/// Running the call under a deadline turns that into an ordinary failed
/// assertion while still exercising the real production path.
fn within_deadline<T: Send + 'static>(
    what: &str,
    action: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(action());
    });
    match receiver.recv_timeout(EVENT_DEADLINE) {
        Ok(value) => value,
        Err(_) => panic!("{what} did not settle within {EVENT_DEADLINE:?}: it must not hang"),
    }
}

/// Builds a GGUF header the way `read_gguf_info` parses it: magic, version,
/// tensor count, metadata count, then `string key`, `u32 value type`, value.
fn gguf_bytes(version: u32, architecture: Option<&str>, file_type: Option<u32>) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"GGUF");
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&1u64.to_le_bytes()); // one tensor
    let entries = u64::from(architecture.is_some()) + u64::from(file_type.is_some());
    bytes.extend_from_slice(&entries.to_le_bytes());
    if let Some(architecture) = architecture {
        push_gguf_string(&mut bytes, "general.architecture");
        bytes.extend_from_slice(&8u32.to_le_bytes()); // GGUF string type
        push_gguf_string(&mut bytes, architecture);
    }
    if let Some(file_type) = file_type {
        push_gguf_string(&mut bytes, "general.file_type");
        bytes.extend_from_slice(&4u32.to_le_bytes()); // GGUF uint32 type
        bytes.extend_from_slice(&file_type.to_le_bytes());
    }
    // Padding keeps the metadata section honest and the file non-empty.
    bytes.resize(bytes.len() + 64, 0);
    bytes
}

fn push_gguf_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

/// A complete configuration pointing at two files.
fn configured(model: &Path, server: &Path) -> LocalAiConfig {
    LocalAiConfig {
        server: LocalModelConfig {
            server_path: server.to_string_lossy().into_owned(),
            model_path: model.to_string_lossy().into_owned(),
            ..LocalModelConfig::default()
        },
        ..LocalAiConfig::default()
    }
}

/// Asserts that shape validation refuses `config` and names `field`.
fn refused_field(config: &LocalAiConfig, field: &str) {
    let issues = config
        .validate_shape()
        .expect_err(&format!("{field} must be refused"));
    assert!(
        issues.iter().any(|issue| issue.field == field),
        "expected an issue naming {field}, got {issues:?}"
    );
}

// ---------------------------------------------------------------------------
// 1. HTTP, streaming, and the failure shapes
// ---------------------------------------------------------------------------

#[test]
fn the_health_endpoint_maps_status_codes_to_states() {
    let script = ProbeScript::new(200, MODELS_WITH_ID, TEMPLATE_WITH_THINKING);
    let server = MockServer::start(script.responder());
    let client = server.client();

    assert_eq!(client.health().unwrap(), HealthState::Ready);
    script.health_status.store(503, Ordering::SeqCst);
    assert_eq!(client.health().unwrap(), HealthState::Loading);
    script.health_status.store(418, Ordering::SeqCst);
    assert_eq!(client.health().unwrap(), HealthState::Unknown);

    // A closed port is a controlled error, not an "unknown" state.
    let closed = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let absent = LocalAiClient::new("127.0.0.1", port).unwrap();
    assert_eq!(absent.health().unwrap_err(), ChatError::ServerNotRunning);
}

#[test]
fn the_probe_reads_the_model_list_and_a_present_chat_template() {
    let script = ProbeScript::new(200, MODELS_WITH_ID, TEMPLATE_WITH_THINKING);
    let server = MockServer::start(script.responder());

    let probe = server.client().probe();

    assert!(probe.reachable);
    assert_eq!(probe.health, Some(HealthState::Ready));
    assert_eq!(probe.model_ids, vec!["mock-qwen3-8b".to_string()]);
    assert_eq!(probe.model_id().as_deref(), Some("mock-qwen3-8b"));
    assert!(probe.chat_template_present);
    assert!(
        probe.thinking_switch_in_template,
        "a template mentioning the switch must be reported as supporting it"
    );
    assert_eq!(probe.build_info.as_deref(), Some("b9999-test"));
    assert!(
        probe.notes.is_empty(),
        "a complete probe needs no caveat: {:?}",
        probe.notes
    );
}

#[test]
fn the_probe_reports_a_missing_chat_template_and_a_missing_model_id() {
    let script = ProbeScript::new(200, MODELS_WITHOUT_ID, TEMPLATE_ABSENT);
    let server = MockServer::start(script.responder());

    let probe = server.client().probe();

    assert!(probe.reachable);
    assert!(probe.model_ids.is_empty());
    assert!(probe.model_id().is_none());
    assert!(!probe.chat_template_present);
    assert!(!probe.thinking_switch_in_template);
    assert_eq!(probe.build_info.as_deref(), Some("b1-test"));
    assert!(
        probe
            .notes
            .iter()
            .any(|note| note.contains("chat template could not be read")),
        "the missing template must be reported: {:?}",
        probe.notes
    );
    assert!(
        probe
            .notes
            .iter()
            .any(|note| note.contains("model identifier")),
        "the missing model id must be reported: {:?}",
        probe.notes
    );
}

#[test]
fn a_complete_sse_stream_yields_tokens_and_a_final_usage_chunk() {
    let server = MockServer::start(sse_responder(complete_stream_pieces()));
    let client = server.client();

    let (result, outcomes) = stream_outcomes(&client, &chat_request(true));

    assert!(result.is_ok(), "the stream must complete: {result:?}");
    assert_eq!(
        outcomes,
        expected_complete_outcomes(),
        "a split data line must become one token, and comments, empty data \
         lines, and data-less events must produce nothing"
    );
}

#[test]
fn a_chunked_sse_stream_produces_the_same_outcomes() {
    // The same bytes, framed in 7-byte chunks: chunk boundaries fall inside
    // lines, inside JSON, and inside the Cyrillic character, so framing bytes
    // must never leak into a line.
    let body: Vec<u8> = complete_stream_pieces().concat();
    let server = MockServer::start(move |request: &MockRequest, stream: &mut TcpStream| {
        if request.path == CHAT_COMPLETIONS_PATH {
            let _ = respond_chunked(stream, "text/event-stream", &body);
        } else {
            let _ = respond_json(stream, 404, b"{}");
        }
    });

    let (result, outcomes) = stream_outcomes(&server.client(), &chat_request(true));

    assert!(result.is_ok(), "a chunked stream must complete: {result:?}");
    assert_eq!(outcomes, expected_complete_outcomes());
}

#[test]
fn a_chunk_with_reasoning_and_text_reports_both_halves() {
    // A server that puts the chain of thought and the answer in the same delta
    // must not lose either half: the reasoning used to be reported while the
    // answer text was silently dropped.
    let pieces = vec![
        b"data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"step by step\",\"content\":\"the answer\"}}]}\n\n".to_vec(),
        b"data: [DONE]\n\n".to_vec(),
    ];
    let server = MockServer::start(sse_responder(pieces));

    let (result, outcomes) = stream_outcomes(&server.client(), &chat_request(true));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        outcomes,
        vec![
            ChunkOutcome::TokenAndThinking {
                text: "the answer".to_string(),
                thinking: "step by step".to_string(),
            },
            ChunkOutcome::Completed {
                finish_reason: None,
                usage: None
            }
        ]
    );
    // The answer half is still answer text for a caller that only wants that.
    assert_eq!(token_text(&outcomes), "the answer");
}

#[test]
fn a_utf8_character_split_across_two_writes_is_reassembled() {
    // The mock stops between the two bytes of "п", so a client that decoded per
    // socket read instead of per line would refuse this stream.
    let event = "data: {\"choices\":[{\"delta\":{\"content\":\"п\"}}]}\n\n";
    let bytes = event.as_bytes();
    let split = bytes.iter().position(|byte| *byte >= 0x80).unwrap() + 1;
    let pieces = vec![
        bytes[..split].to_vec(),
        bytes[split..].to_vec(),
        b"data: [DONE]\n\n".to_vec(),
    ];
    let server = MockServer::start(sse_responder(pieces));

    let (result, outcomes) = stream_outcomes(&server.client(), &chat_request(true));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        outcomes,
        vec![
            ChunkOutcome::Token("п".to_string()),
            ChunkOutcome::Completed {
                finish_reason: None,
                usage: None
            }
        ]
    );
}

#[test]
fn a_done_marker_completes_the_stream_without_usage() {
    let pieces = vec![
        b"data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\n".to_vec(),
        b"data: [DONE]\n\n".to_vec(),
    ];
    let server = MockServer::start(sse_responder(pieces));

    let (result, outcomes) = stream_outcomes(&server.client(), &chat_request(true));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(outcomes.len(), 2, "got {outcomes:?}");
    assert_eq!(outcomes[0], ChunkOutcome::Token("done".to_string()));
    assert!(matches!(outcomes[1], ChunkOutcome::Completed { .. }));
    assert_eq!(completion(&outcomes), Some((None, None)));
}

#[test]
fn the_three_response_framings_are_all_read() {
    let server = MockServer::start(|request: &MockRequest, stream: &mut TcpStream| {
        match request.path.as_str() {
            "/framed/length" => {
                let body = b"{\"framing\":\"length\"}";
                let _ = respond_with_length(stream, 200, "application/json", body.len(), body);
            }
            "/framed/overrun" => {
                // Declares five bytes but sends ten: the declaration wins.
                let _ = respond_with_length(stream, 200, "application/json", 5, b"helloEXTRA");
            }
            "/framed/chunked" => {
                let _ = respond_chunked(stream, "application/json", b"{\"framing\":\"chunked\"}");
            }
            "/framed/close" => {
                let _ = respond_until_close(stream, "application/json", b"{\"framing\":\"close\"}");
            }
            _ => {
                let _ = respond_json(stream, 404, b"{}");
            }
        }
    });
    let endpoint = LoopbackEndpoint::new("127.0.0.1", server.port()).unwrap();
    assert!(endpoint.is_loopback());
    assert_eq!(endpoint.address().port(), server.port());

    // Content-Length, read through the head and the body reader directly.
    let response = endpoint
        .request("GET", "/framed/length", "application/json", None)
        .unwrap();
    assert!(response.head.is_success());
    assert_eq!(
        response.head.header("content-type"),
        Some("application/json")
    );
    assert_eq!(
        response.into_body().read_to_end().unwrap(),
        b"{\"framing\":\"length\"}"
    );

    // A declared length bounds the body, whatever else the peer sends.
    let (status, body) = endpoint.get_json("/framed/overrun").unwrap();
    assert_eq!(status, 200);
    assert_eq!(body, b"hello", "the declared length must bound the body");

    // Chunked.
    let (status, body) = endpoint.get_json("/framed/chunked").unwrap();
    assert_eq!(status, 200);
    assert_eq!(body, b"{\"framing\":\"chunked\"}");

    // Close-delimited.
    let (status, body) = endpoint.get_json("/framed/close").unwrap();
    assert_eq!(status, 200);
    assert_eq!(body, b"{\"framing\":\"close\"}");
}

#[test]
fn a_non_streaming_completion_returns_the_message_content() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&captured);
    let server = MockServer::start(move |request: &MockRequest, stream: &mut TcpStream| {
        recorder.lock().unwrap().push(request.clone());
        if request.path == CHAT_COMPLETIONS_PATH {
            let body = br#"{"choices":[{"message":{"role":"assistant","content":"mock answer"}}]}"#;
            let _ = respond_with_length(stream, 200, "application/json", body.len(), body);
        } else {
            let _ = respond_json(stream, 404, b"{}");
        }
    });
    let client = server.client();

    let answer = client.chat_once(&chat_request(true)).unwrap();

    assert_eq!(answer, "mock answer");
    let requests = captured.lock().unwrap();
    let sent = requests
        .first()
        .expect("the mock must have received a request");
    assert_eq!(sent.method, "POST");
    assert_eq!(sent.path, CHAT_COMPLETIONS_PATH);
    // The non-streaming call must clear the streaming fields itself.
    let body = sent.json();
    assert_eq!(body["stream"], serde_json::json!(false));
    assert!(
        body.get("stream_options").is_none(),
        "a non-streaming request must not ask for stream options: {body}"
    );
}

#[test]
fn an_http_error_status_is_reported_without_the_body() {
    const ECHOED_PROMPT: &str = "FICTIONAL_PROMPT_ECHOED_BY_THE_SERVER";
    let status = Arc::new(AtomicU16::new(400));
    let responder = {
        let status = Arc::clone(&status);
        move |request: &MockRequest, stream: &mut TcpStream| {
            if request.path == CHAT_COMPLETIONS_PATH {
                let body = format!("{{\"error\":\"{ECHOED_PROMPT}\"}}");
                let current = status.load(Ordering::SeqCst);
                let _ = respond_with_length(
                    stream,
                    current,
                    "application/json",
                    body.len(),
                    body.as_bytes(),
                );
            } else {
                let _ = respond_json(stream, 200, b"{}");
            }
        }
    };
    let server = MockServer::start(responder);
    let client = server.client();

    // A 4xx on a streaming request is an HTTP status error with no outcomes.
    let (result, outcomes) = stream_outcomes(&client, &chat_request(true));
    assert_eq!(result.unwrap_err(), ChatError::HttpStatus(400));
    assert!(outcomes.is_empty(), "a refused request produces no chunk");

    // A 5xx is reported the same way, and the body never reaches the error.
    status.store(500, Ordering::SeqCst);
    let error = client.chat_once(&chat_request(false)).unwrap_err();
    assert_eq!(error, ChatError::HttpStatus(500));
    let rendered = format!("{error} {error:?}");
    assert!(
        !rendered.contains(ECHOED_PROMPT),
        "an error must never carry the server body: {rendered}"
    );
}

#[test]
fn a_malformed_chunk_is_ignored_when_real_tokens_arrive() {
    let pieces = vec![
        b"data: {not json\n\n".to_vec(),
        b"data: {\"choices\":[{\"delta\":{\"content\":\"usable\"}}]}\n\n".to_vec(),
        b"data: [DONE]\n\n".to_vec(),
    ];
    let server = MockServer::start(sse_responder(pieces));

    let (result, outcomes) = stream_outcomes(&server.client(), &chat_request(true));

    assert!(result.is_ok(), "one bad chunk must not destroy the answer");
    assert_eq!(token_text(&outcomes), "usable");
}

#[test]
fn a_stream_that_only_contains_a_malformed_chunk_is_an_error() {
    let pieces = vec![b"data: {not json\n\n".to_vec()];
    let server = MockServer::start(sse_responder(pieces));

    let (result, outcomes) = stream_outcomes(&server.client(), &chat_request(true));

    assert_eq!(
        result.unwrap_err(),
        ChatError::InvalidStream,
        "a stream with no usable token and no parseable chunk is a failure"
    );
    assert!(outcomes.is_empty());
}

#[test]
fn a_truncated_stream_without_a_complete_event_is_an_error() {
    // The head promises 512 bytes and the peer closes after a partial data line:
    // the client must report a malformed stream, not a silent empty answer.
    let server = MockServer::start(|request: &MockRequest, stream: &mut TcpStream| {
        if request.path == CHAT_COMPLETIONS_PATH {
            let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 512\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(head);
            let _ = stream.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"trunc");
            let _ = stream.flush();
        } else {
            let _ = respond_json(stream, 404, b"{}");
        }
    });

    let (result, outcomes) = stream_outcomes(&server.client(), &chat_request(true));

    assert_eq!(result.unwrap_err(), ChatError::InvalidStream);
    assert!(outcomes.is_empty());
}

#[test]
fn a_truncated_stream_still_delivers_the_tokens_it_produced() {
    // Truncation is only an error when nothing usable arrived: everything the
    // server managed to send is kept.
    let server = MockServer::start(|request: &MockRequest, stream: &mut TcpStream| {
        if request.path == CHAT_COMPLETIONS_PATH {
            let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 512\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(head);
            let _ =
                stream.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n");
            let _ =
                stream.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"second\"}}]}\n\n");
            let _ = stream.flush();
        } else {
            let _ = respond_json(stream, 404, b"{}");
        }
    });

    let (result, outcomes) = stream_outcomes(&server.client(), &chat_request(true));

    assert!(result.is_ok(), "a usable answer must survive truncation");
    assert_eq!(token_text(&outcomes), "firstsecond");
}

#[test]
fn a_stalled_stream_times_out() {
    let server = MockServer::start(|request: &MockRequest, stream: &mut TcpStream| {
        if request.path == CHAT_COMPLETIONS_PATH {
            let head = b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 4096\r\nConnection: close\r\n\r\n";
            let _ = stream.write_all(head);
            let _ = stream
                .write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"stalled\"}}]}\n\n");
            let _ = stream.flush();
            std::thread::sleep(SOCKET_TIMEOUT);
        } else {
            let _ = respond_json(stream, 404, b"{}");
        }
    });

    // The production client waits 90 seconds before declaring a stall, so this
    // test drives the same reader through the endpoint with a test-sized
    // deadline. The stall logic itself is the production one.
    let endpoint = LoopbackEndpoint::new("127.0.0.1", server.port())
        .unwrap()
        .with_timeouts(
            Duration::from_millis(500),
            Duration::from_millis(20),
            Duration::from_millis(200),
        );
    let response = endpoint
        .request(
            "POST",
            CHAT_COMPLETIONS_PATH,
            "text/event-stream",
            Some("{}"),
        )
        .unwrap();
    assert_eq!(response.head.status, 200);

    let mut body: BodyReader<TcpStream> = response.into_body();
    let cancel = cancellation_flag();
    let started = Instant::now();
    assert_eq!(
        body.next_line(&cancel).unwrap().unwrap(),
        "data: {\"choices\":[{\"delta\":{\"content\":\"stalled\"}}]}"
    );
    assert_eq!(body.next_line(&cancel).unwrap().unwrap(), "");
    let error = body.next_line(&cancel).unwrap_err();

    assert_eq!(
        error,
        ChatError::TimedOut,
        "a stream that stops sending must time out"
    );
    assert!(
        started.elapsed() < SOCKET_TIMEOUT,
        "the short stall deadline must be the one that fires"
    );
}

#[test]
fn a_body_larger_than_the_configured_maximum_is_refused() {
    let server = MockServer::start(|_request: &MockRequest, stream: &mut TcpStream| {
        let declared = MAX_BODY_BYTES + 4 * 1024 * 1024;
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {declared}\r\nConnection: close\r\n\r\n"
        );
        let _ = stream.write_all(head.as_bytes());
        let filler = vec![b'x'; 64 * 1024];
        let mut written = 0u64;
        while written < declared {
            // The client hangs up once it refuses the body.
            if stream.write_all(&filler).is_err() {
                break;
            }
            written += filler.len() as u64;
        }
        let _ = stream.flush();
    });
    let endpoint = LoopbackEndpoint::new("127.0.0.1", server.port()).unwrap();

    let started = Instant::now();
    let error = endpoint.get_json("/v1/models").unwrap_err();

    assert_eq!(
        error,
        ChatError::InvalidStream,
        "a body over {MAX_BODY_BYTES} bytes must be refused"
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the size bound must stop the read early"
    );
}

#[test]
fn cancelling_mid_stream_returns_promptly_from_another_thread() {
    let server = MockServer::start(move |request: &MockRequest, stream: &mut TcpStream| {
        if request.path == CHAT_COMPLETIONS_PATH {
            let _ = respond_tokens_then_stall(stream);
        } else {
            let _ = respond_json(stream, 404, b"{}");
        }
    });
    let client = server.client();
    let request = chat_request(true);

    let cancel = Arc::new(cancellation_flag());
    let canceller = {
        let flag = Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            request_cancel(&flag)
        })
    };

    let started = Instant::now();
    let mut outcomes = Vec::new();
    let result = client.stream_chat(&request, &cancel, &mut |outcome| outcomes.push(outcome));
    let elapsed = started.elapsed();
    canceller.join().unwrap();

    assert_eq!(result.unwrap_err(), ChatError::Cancelled);
    assert!(
        !outcomes.is_empty(),
        "the tokens that arrived before the cancellation belong to the caller"
    );
    assert_eq!(token_text(&outcomes), "half ");
    assert!(
        elapsed < Duration::from_secs(2),
        "cancellation must not wait for the server to send more: took {elapsed:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. Configuration and model validation
// ---------------------------------------------------------------------------

#[test]
fn a_synthetic_gguf_file_passes_validation_and_reports_its_declared_metadata() {
    let directory = tempdir().unwrap();
    let server_path = directory.path().join("llama-server.exe");
    std::fs::write(&server_path, b"MZ").unwrap();
    let model_path = directory.path().join("Qwen3-8B-Q4_K_M.gguf");
    let mut bytes = gguf_bytes(3, Some("qwen3"), Some(15));
    // Real model files are large; the warning threshold is one mebibyte.
    bytes.resize(2 * 1024 * 1024, 0);
    std::fs::write(&model_path, &bytes).unwrap();

    let config = configured(&model_path, &server_path);
    let validation = validate_model(&config);

    assert_eq!(
        validation.level,
        CheckLevel::Ok,
        "issues: {:?}",
        validation.issues
    );
    assert!(!validation.is_blocked());
    let model = validation.model.expect("a readable model must be reported");
    assert_eq!(model.file_name, "Qwen3-8B-Q4_K_M.gguf");
    assert_eq!(model.size_bytes, bytes.len() as u64);
    assert_eq!(model.gguf.version, 3);
    assert_eq!(model.gguf.tensor_count, 1);
    assert_eq!(model.gguf.architecture.as_deref(), Some("qwen3"));
    assert_eq!(model.gguf.quantisation.as_deref(), Some("Q4_K_M"));
    assert!(model.gguf.metadata_read);
    assert_eq!(validation.server_file.as_deref(), Some("llama-server.exe"));

    // The file-level check and the full start check both accept it.
    assert!(validate_files(&config).is_ok());
    assert!(config.validate_for_start().is_ok());
}

#[test]
fn non_gguf_missing_and_misnamed_files_are_refused() {
    let directory = tempdir().unwrap();
    let server_path = directory.path().join("llama-server.exe");
    std::fs::write(&server_path, b"MZ").unwrap();

    // A file that is not a GGUF model at all.
    let impostor = directory.path().join("impostor.gguf");
    std::fs::write(&impostor, b"this is not a model").unwrap();
    let validation = validate_model(&configured(&impostor, &server_path));
    assert!(validation.is_blocked());
    assert!(
        validation
            .issues
            .iter()
            .any(|issue| issue.message.contains("GGUF magic")),
        "issues: {:?}",
        validation.issues
    );

    // Too short to hold a header.
    let tiny = directory.path().join("tiny.gguf");
    std::fs::write(&tiny, b"GG").unwrap();
    assert!(jarvis_core::ai::local::read_gguf_info(&tiny)
        .unwrap_err()
        .contains("too short"));

    // A GGUF version this build refuses.
    let future = directory.path().join("future.gguf");
    let mut bytes = gguf_bytes(9, Some("qwen3"), None);
    bytes[4..8].copy_from_slice(&9u32.to_le_bytes());
    std::fs::write(&future, &bytes).unwrap();
    assert!(jarvis_core::ai::local::read_gguf_info(&future)
        .unwrap_err()
        .contains("version"));

    // A valid model with the wrong file name.
    let misnamed = directory.path().join("model.bin");
    std::fs::write(&misnamed, gguf_bytes(3, Some("qwen3"), None)).unwrap();
    let validation = validate_model(&configured(&misnamed, &server_path));
    assert!(validation.is_blocked());
    assert!(
        validation
            .issues
            .iter()
            .any(|issue| issue.message.contains(".gguf")),
        "issues: {:?}",
        validation.issues
    );

    // A model file that does not exist.
    let missing = configured(&directory.path().join("absent.gguf"), &server_path);
    let validation = validate_model(&missing);
    assert!(validation.is_blocked());
    assert!(
        validation
            .issues
            .iter()
            .any(|issue| issue.field == "model_path" && issue.message.contains("does not exist")),
        "issues: {:?}",
        validation.issues
    );
    assert!(matches!(
        missing.validate_for_start().unwrap_err(),
        ChatError::ModelUnavailable(_)
    ));

    // A server executable that does not exist.
    let model_path = directory.path().join("model.gguf");
    std::fs::write(&model_path, gguf_bytes(3, Some("qwen3"), None)).unwrap();
    let validation = validate_model(&configured(
        &model_path,
        &directory.path().join("absent.exe"),
    ));
    assert!(validation.is_blocked());
    assert!(validation
        .issues
        .iter()
        .any(|issue| issue.field == "server_path" && issue.level == CheckLevel::Blocked));
}

#[test]
fn server_arguments_keep_hostile_paths_in_a_single_argument() {
    let base = configured(
        Path::new("C:/models/Qwen3-8B-Q4_K_M.gguf"),
        Path::new("C:/tools/llama-server.exe"),
    );
    let baseline = base.server_arguments().unwrap();
    assert_eq!(baseline[0], "--model");
    assert_eq!(baseline[1], "C:/models/Qwen3-8B-Q4_K_M.gguf");
    assert_eq!(baseline.len(), 8, "arguments: {baseline:?}");
    // Zero means "leave it to llama.cpp", so no flag is added.
    assert!(!baseline.contains(&"--threads".to_string()));
    assert!(!baseline.contains(&"--n-gpu-layers".to_string()));

    // Spaces and shell metacharacters in the path must not become more
    // arguments, quoting, escaping, or a shell command.
    let hostile = "C:/my models/qwen 8b & calc | echo > out; $(boom) `run` %PATH% \"quoted\".gguf";
    let mut config = base.clone();
    config.server.model_path = hostile.to_string();
    let arguments = config.server_arguments().unwrap();

    assert_eq!(
        arguments.len(),
        baseline.len(),
        "spaces must not add arguments: {arguments:?}"
    );
    assert_eq!(arguments[1], hostile, "the path is passed verbatim");
    let model_flag = arguments
        .iter()
        .position(|argument| argument == "--model")
        .expect("--model must be present");
    assert_eq!(arguments[model_flag + 1], hostile);
    assert_eq!(
        arguments
            .iter()
            .filter(|argument| *argument == hostile)
            .count(),
        1
    );
    // The path is never wrapped in quotes or escaped for a shell. (The hostile
    // path intentionally contains quotes of its own, so this compares against
    // the quoted and escaped spellings a shell string would need.)
    let quoted = format!("\"{hostile}\"");
    let escaped = hostile.replace('"', "\\\"");
    assert!(
        arguments
            .iter()
            .all(|argument| *argument != quoted && *argument != escaped),
        "nothing may be quoted or escaped for a shell: {arguments:?}"
    );

    // A path that only contains spaces and other metacharacters gains neither
    // quotes nor escapes either.
    let spaces_only = "C:/my models/qwen 8b (q4).gguf";
    let mut plain = base.clone();
    plain.server.model_path = spaces_only.to_string();
    let arguments = plain.server_arguments().unwrap();
    assert_eq!(arguments[1], spaces_only);
    assert!(
        arguments
            .iter()
            .all(|argument| !argument.contains('"') && !argument.contains('\'')),
        "a path with spaces must not be quoted: {arguments:?}"
    );

    // Optional overrides stay separate arguments too.
    let mut overrides = base.clone();
    overrides.server.cpu_threads = 6;
    overrides.server.gpu_layers = 20;
    let arguments = overrides.server_arguments().unwrap();
    assert!(arguments.contains(&"--threads".to_string()));
    assert!(arguments.contains(&"6".to_string()));
    assert!(arguments.contains(&"--n-gpu-layers".to_string()));
    assert!(arguments.contains(&"20".to_string()));

    // Without a model path there is nothing to run.
    let mut unconfigured = base.clone();
    unconfigured.server.model_path = "   ".to_string();
    assert!(unconfigured.server_arguments().is_err());
}

#[test]
fn shape_validation_refuses_non_loopback_hosts_and_out_of_range_values() {
    let base = configured(
        Path::new("C:/models/Qwen3-8B-Q4_K_M.gguf"),
        Path::new("C:/tools/llama-server.exe"),
    );
    assert!(
        base.validate_shape().is_ok(),
        "the baseline must be valid: {:?}",
        base.validate_shape()
    );

    // Loopback spellings are accepted.
    for host in ["127.0.0.1", "localhost", "::1"] {
        let mut config = base.clone();
        config.server.host = host.to_string();
        assert!(
            config.validate_shape().is_ok(),
            "{host} is a loopback host: {:?}",
            config.validate_shape()
        );
    }
    // Everything else is refused, naming the field for the interface.
    for host in ["0.0.0.0", "::", "192.168.1.10", "10.0.0.1", "example.com"] {
        let mut config = base.clone();
        config.server.host = host.to_string();
        let issues = config.validate_shape().unwrap_err();
        assert!(
            issues.iter().any(|issue| issue.field == "host"),
            "{host} must be refused: {issues:?}"
        );
    }

    // A LAN request is refused on its own field as well.
    let mut lan = base.clone();
    lan.allow_lan = true;
    refused_field(&lan, "allow_lan");

    let mut port = base.clone();
    port.server.port = 80;
    refused_field(&port, "port");

    let mut small_context = base.clone();
    small_context.server.context_size = 16;
    refused_field(&small_context, "context_size");

    let mut huge_context = base.clone();
    huge_context.server.context_size = 262_145;
    refused_field(&huge_context, "context_size");

    let mut short_timeout = base.clone();
    short_timeout.server.startup_timeout_seconds = 4;
    refused_field(&short_timeout, "startup_timeout_seconds");

    let mut long_timeout = base.clone();
    long_timeout.server.startup_timeout_seconds = 901;
    refused_field(&long_timeout, "startup_timeout_seconds");

    let mut hot = base.clone();
    hot.temperature = 9.0;
    refused_field(&hot, "temperature");

    let mut not_a_number = base.clone();
    not_a_number.temperature = f32::NAN;
    refused_field(&not_a_number, "temperature");

    let mut no_nucleus = base.clone();
    no_nucleus.top_p = 0.0;
    refused_field(&no_nucleus, "top_p");

    let mut wide_nucleus = base.clone();
    wide_nucleus.top_p = 1.5;
    refused_field(&wide_nucleus, "top_p");

    let mut no_tokens = base.clone();
    no_tokens.max_tokens = 0;
    refused_field(&no_tokens, "max_tokens");

    let mut many_tokens = base.clone();
    many_tokens.max_tokens = 40_000;
    refused_field(&many_tokens, "max_tokens");

    let mut many_threads = base.clone();
    many_threads.server.cpu_threads = 300;
    refused_field(&many_threads, "cpu_threads");

    let mut many_layers = base.clone();
    many_layers.server.gpu_layers = 2_000;
    refused_field(&many_layers, "gpu_layers");

    let mut future_schema = base.clone();
    future_schema.schema_version = 99;
    refused_field(&future_schema, "schema_version");

    // Incomplete and unusable configurations name the missing setting, and the
    // start check turns them into a controlled error rather than a panic.
    let mut no_server = base.clone();
    no_server.server.server_path = "  ".to_string();
    refused_field(&no_server, "server_path");

    let mut no_model = base.clone();
    no_model.server.model_path = String::new();
    refused_field(&no_model, "model_path");

    let error = no_model.validate_for_start().unwrap_err();
    assert!(
        matches!(error, ChatError::InvalidConfiguration(_)),
        "got {error:?}"
    );
    assert!(
        !error.to_string().contains("C:/"),
        "a configuration error must not echo paths: {error}"
    );
}

// ---------------------------------------------------------------------------
// 3. Security properties
// ---------------------------------------------------------------------------

#[test]
fn a_client_and_an_endpoint_refuse_a_non_loopback_host() {
    assert!(LocalAiClient::new("127.0.0.1", 8080).is_ok());
    assert!(LocalAiClient::new("localhost", 8080).is_ok());
    assert!(LocalAiClient::new("::1", 8080).is_ok());
    let client = LocalAiClient::new("127.0.0.1", 8080).unwrap();
    assert!(client.endpoint().is_loopback());
    assert!(client.endpoint().address().ip().is_loopback());

    for host in [
        "0.0.0.0",
        "::",
        "192.168.1.10",
        "10.0.0.1",
        "example.com",
        "8.8.8.8",
        "",
    ] {
        assert!(
            LocalAiClient::new(host, 8080).is_err(),
            "{host} must be refused by the client"
        );
        assert!(
            LoopbackEndpoint::new(host, 8080).is_err(),
            "{host} must be refused by the endpoint"
        );
        assert!(
            loopback_address(host, 8080).is_err(),
            "{host} must be refused before a socket is opened"
        );
    }

    // The refusal is a configuration error, so the interface can show it.
    assert!(matches!(
        loopback_address("192.168.1.10", 8080).unwrap_err(),
        ChatError::InvalidConfiguration(_)
    ));
}

#[test]
fn the_chat_request_on_the_wire_carries_no_credential_field() {
    let captured = Arc::new(Mutex::new(Vec::new()));
    let server = MockServer::start(capturing_responder(
        Arc::clone(&captured),
        complete_stream_pieces(),
    ));
    let client = server.client();

    let (result, _) = stream_outcomes(&client, &chat_request(true));
    assert!(result.is_ok(), "{result:?}");

    let requests = captured.lock().unwrap();
    let request = requests
        .first()
        .expect("the mock must have received the chat request");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, CHAT_COMPLETIONS_PATH);
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert!(
        request
            .header("host")
            .map(|host| host.ends_with(&server.port().to_string()))
            .unwrap_or(false),
        "the client must address the managed server: {:?}",
        request.header("host")
    );

    // No credential is ever sent as a header.
    for forbidden in [
        "authorization",
        "proxy-authorization",
        "cookie",
        "x-api-key",
        "api-key",
    ] {
        assert_eq!(
            request.header(forbidden),
            None,
            "the client must not send a {forbidden} header"
        );
    }

    // The body carries exactly the OpenAI fields the client needs, checked as
    // parsed fields rather than as substrings.
    let body = request.json();
    let object = body.as_object().expect("the body must be a JSON object");
    let allowed = [
        "model",
        "messages",
        "stream",
        "max_tokens",
        "temperature",
        "top_p",
        "chat_template_kwargs",
        "stream_options",
    ];
    for key in object.keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "unexpected request field on the wire: {key}"
        );
    }
    for forbidden in [
        "api_key",
        "apikey",
        "authorization",
        "password",
        "master_key",
        "secret",
        "notes",
        "storage",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "the chat request must not carry a {forbidden} field"
        );
    }

    // Every message carries a role and content, and nothing else.
    let messages = object["messages"]
        .as_array()
        .expect("messages must be a list");
    assert_eq!(messages.len(), 2);
    for message in messages {
        let message = message.as_object().unwrap();
        assert_eq!(message.len(), 2, "messages carry only role and content");
        assert!(message.contains_key("role") && message.contains_key("content"));
    }
    // The question that crossed the socket is the user's own text.
    assert_eq!(
        messages[1]["content"],
        serde_json::json!("FICTIONAL_USER_QUESTION")
    );
}

// ---------------------------------------------------------------------------
// 4. Gateway lifecycle with a fake process runner
// ---------------------------------------------------------------------------

/// A fake `llama-server` process: it owns the mock HTTP server that answers the
/// gateway and reports the lifecycle the gateway asks about.
struct FakeServerProcess {
    /// Taken on kill: dropping the mock server closes the loopback port, exactly
    /// as terminating the child would.
    server: Mutex<Option<MockServer>>,
    /// Shared with the test, so the assertion can see the kill.
    killed: Arc<AtomicBool>,
    /// How often the gateway polled this process; proves `try_wait` is used.
    polls: Arc<AtomicUsize>,
}

impl ServerProcess for FakeServerProcess {
    fn pid(&self) -> Option<u32> {
        Some(4242)
    }

    fn try_wait(&mut self) -> Result<Option<ServerExit>, ChatError> {
        self.polls.fetch_add(1, Ordering::SeqCst);
        if self.killed.load(Ordering::SeqCst) {
            Ok(Some(ServerExit {
                code: Some(0),
                success: true,
            }))
        } else {
            Ok(None)
        }
    }

    fn kill(&mut self) -> Result<(), ChatError> {
        self.killed.store(true, Ordering::SeqCst);
        let _ = self.server.lock().unwrap().take();
        Ok(())
    }
}

/// A runner that hands out fake processes and records what it was asked to spawn.
struct FakeProcessRunner {
    /// The mock server the next spawned process will own.
    server: Mutex<Option<MockServer>>,
    program: Mutex<Option<PathBuf>>,
    arguments: Mutex<Vec<String>>,
    killed: Arc<AtomicBool>,
    polls: Arc<AtomicUsize>,
    spawns: AtomicUsize,
}

impl FakeProcessRunner {
    fn new(server: MockServer) -> Arc<Self> {
        Arc::new(Self {
            server: Mutex::new(Some(server)),
            program: Mutex::new(None),
            arguments: Mutex::new(Vec::new()),
            killed: Arc::new(AtomicBool::new(false)),
            polls: Arc::new(AtomicUsize::new(0)),
            spawns: AtomicUsize::new(0),
        })
    }

    fn was_killed(&self) -> bool {
        self.killed.load(Ordering::SeqCst)
    }

    fn spawn_count(&self) -> usize {
        self.spawns.load(Ordering::SeqCst)
    }

    fn recorded_program(&self) -> Option<PathBuf> {
        self.program.lock().unwrap().clone()
    }

    fn recorded_arguments(&self) -> Vec<String> {
        self.arguments.lock().unwrap().clone()
    }
}

impl ProcessRunner for FakeProcessRunner {
    fn spawn(&self, program: &Path, arguments: &[String]) -> Result<SpawnedServer, ChatError> {
        self.spawns.fetch_add(1, Ordering::SeqCst);
        *self.program.lock().unwrap() = Some(program.to_path_buf());
        *self.arguments.lock().unwrap() = arguments.to_vec();
        let server = self
            .server
            .lock()
            .unwrap()
            .take()
            .ok_or(ChatError::ProcessUnavailable)?;
        Ok(SpawnedServer {
            process: Box::new(FakeServerProcess {
                server: Mutex::new(Some(server)),
                killed: Arc::clone(&self.killed),
                polls: Arc::clone(&self.polls),
            }),
            // No stderr stream: the fake never writes diagnostics.
            stderr: None,
            program_label: "fake-llama-server".to_string(),
        })
    }
}

/// Writes a synthetic GGUF model file and a stand-in server executable, so
/// validation passes without a real model.
fn write_fixture_files(directory: &Path) -> (PathBuf, PathBuf) {
    let server_path = directory.join("llama-server.exe");
    std::fs::write(&server_path, b"MZ").unwrap();
    let model_path = directory.join("Qwen3-8B-Q4_K_M.gguf");
    let mut bytes = gguf_bytes(3, Some("qwen3"), Some(15));
    bytes.resize(2 * 1024 * 1024, 0);
    std::fs::write(&model_path, &bytes).unwrap();
    (server_path, model_path)
}

#[test]
fn the_gateway_lifecycle_streams_cancels_and_stops_with_a_fake_process() {
    let directory = tempdir().unwrap();
    let (server_path, model_path) = write_fixture_files(directory.path());

    // The mock server answers the probes and both kinds of chat request. It is
    // bound before the gateway is built, because the configuration needs its
    // port.
    let chat_requests = Arc::new(AtomicUsize::new(0));
    let responder = {
        let chat_requests = Arc::clone(&chat_requests);
        move |request: &MockRequest, stream: &mut TcpStream| match request.path.as_str() {
            HEALTH_PATH => {
                let _ = respond_json(stream, 200, b"{\"status\":\"ok\"}");
            }
            MODELS_PATH => {
                let _ = respond_json(stream, 200, MODELS_WITH_ID.as_bytes());
            }
            PROPS_PATH => {
                let _ = respond_json(stream, 200, TEMPLATE_WITH_THINKING.as_bytes());
            }
            CHAT_COMPLETIONS_PATH => {
                if request.json()["stream"] == serde_json::json!(false) {
                    let body =
                        br#"{"choices":[{"message":{"role":"assistant","content":"mock blocking answer"}}]}"#;
                    let _ = respond_with_length(stream, 200, "application/json", body.len(), body);
                } else {
                    match chat_requests.fetch_add(1, Ordering::SeqCst) {
                        // A complete answer, with usage and a finish reason.
                        0 => {
                            let _ = respond_sse(stream, &complete_stream_pieces());
                        }
                        // One delta that carries reasoning and answer text.
                        1 => {
                            let _ = respond_sse(stream, &reasoning_stream_pieces());
                        }
                        // Tokens, then silence: only cancellation ends this.
                        _ => {
                            let _ = respond_tokens_then_stall(stream);
                        }
                    }
                }
            }
            _ => {
                let _ = respond_json(stream, 404, b"{}");
            }
        }
    };
    let runner = FakeProcessRunner::new(MockServer::start(responder));
    let port = runner
        .server
        .lock()
        .unwrap()
        .as_ref()
        .expect("the mock server must still be there")
        .port();

    let config = LocalAiConfig {
        server: LocalModelConfig {
            server_path: server_path.to_string_lossy().into_owned(),
            model_path: model_path.to_string_lossy().into_owned(),
            port,
            context_size: 512,
            startup_timeout_seconds: 10,
            ..LocalModelConfig::default()
        },
        ..LocalAiConfig::default()
    };
    let runner_trait: Arc<dyn ProcessRunner> = runner.clone();
    // Shared, so the stop assertions can also run on a worker thread under a
    // deadline and a lock regression fails instead of hanging the binary.
    let gateway = Arc::new(LocalAiGateway::with_runner(config.clone(), runner_trait));

    // --- start ---
    let status = gateway
        .start()
        .expect("the fake server answers /health immediately");
    assert_eq!(status.state, LocalAiState::Ready);
    assert_eq!(status.pid, Some(4242));
    assert_eq!(status.port, port);
    assert!(status.capabilities.endpoint_available);
    assert!(status.capabilities.streaming);
    assert_eq!(
        status.capabilities.model_id.as_deref(),
        Some("mock-qwen3-8b")
    );
    assert!(
        status.capabilities.thinking_switch,
        "the template mentions the thinking switch"
    );
    assert_eq!(status.model_file.as_deref(), Some("Qwen3-8B-Q4_K_M.gguf"));
    assert!(!status.generating);

    // Exactly one process was spawned, with the model path as its own argument.
    assert_eq!(runner.spawn_count(), 1);
    assert_eq!(
        runner.recorded_program().as_deref(),
        Some(server_path.as_path())
    );
    assert_eq!(
        runner.recorded_arguments(),
        config.server_arguments().unwrap()
    );

    // Starting again is idempotent: no second process, still ready.
    let again = gateway.start().unwrap();
    assert_eq!(again.state, LocalAiState::Ready);
    assert_eq!(runner.spawn_count(), 1);

    // --- streaming generation ---
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = event_sink(&events);
    let _handle = gateway
        .start_generation(
            GenerationRequest::user_message("FICTIONAL_USER_QUESTION"),
            Arc::clone(&sink),
        )
        .expect("a ready gateway accepts a generation");

    wait_for(&events, |events| {
        events
            .iter()
            .any(|event| matches!(event, GenerationEvent::Completed { .. }))
    });

    let collected = events.lock().unwrap().clone();
    assert!(
        matches!(
            collected.first(),
            Some(GenerationEvent::Started {
                stream: true,
                profile: Persona::Jarvis,
                thinking_applied: false,
                ..
            })
        ),
        "the first event must describe the generation: {:?}",
        event_kinds(&collected)
    );
    assert_eq!(
        collected
            .iter()
            .filter_map(|event| match event {
                GenerationEvent::Token { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>(),
        "Helloпривет!"
    );
    let completed = collected
        .iter()
        .find_map(|event| match event {
            GenerationEvent::Completed {
                usage,
                finish_reason,
                cancelled,
                ..
            } => Some((*usage, finish_reason.clone(), *cancelled)),
            _ => None,
        })
        .expect("a completed event");
    assert_eq!(
        completed.0,
        Some(GenerationUsage {
            prompt_tokens: 11,
            completion_tokens: 4,
            total_tokens: 15
        })
    );
    assert_eq!(completed.1.as_deref(), Some("stop"));
    assert!(!completed.2);
    assert!(!gateway.status().unwrap().generating);
    assert_eq!(gateway.state(), LocalAiState::Ready);

    // --- a delta that carries reasoning and answer text together ---
    events.lock().unwrap().clear();
    let _ = gateway
        .start_generation(
            GenerationRequest::user_message("FICTIONAL_REASONING_QUESTION"),
            Arc::clone(&sink),
        )
        .unwrap();
    wait_for(&events, |events| {
        events
            .iter()
            .any(|event| matches!(event, GenerationEvent::Completed { .. }))
    });
    let collected = events.lock().unwrap().clone();
    let reported: Vec<(&str, String)> = collected
        .iter()
        .filter_map(|event| match event {
            GenerationEvent::Thinking { text } => Some(("thinking", text.clone())),
            GenerationEvent::Token { text } => Some(("token", text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        reported,
        vec![
            ("thinking", "step by step".to_string()),
            ("token", "the answer".to_string()),
        ],
        "one delta with both fields must report the reasoning first and then \
         the answer text: {:?}",
        event_kinds(&collected)
    );

    // --- cancellation ---
    events.lock().unwrap().clear();
    let handle = gateway
        .start_generation(
            GenerationRequest::user_message("FICTIONAL_SECOND_QUESTION"),
            Arc::clone(&sink),
        )
        .unwrap();
    wait_for(&events, |events| {
        events
            .iter()
            .any(|event| matches!(event, GenerationEvent::Token { .. }))
    });

    assert!(
        gateway.cancel(),
        "a running generation must accept cancellation"
    );
    assert!(handle.is_cancelled());
    wait_for(&events, |events| {
        events
            .iter()
            .any(|event| matches!(event, GenerationEvent::Cancelled { .. }))
    });
    let cancelled = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            GenerationEvent::Cancelled { partial, .. } => Some(*partial),
            _ => None,
        })
        .unwrap();
    assert!(cancelled, "some text had already been produced");
    // The flag is cleared once the worker settles, so a second cancel is a no-op.
    assert!(!gateway.cancel());
    assert_eq!(gateway.state(), LocalAiState::Ready);

    // --- non-streaming generation, still through the gateway ---
    events.lock().unwrap().clear();
    let outcome = gateway
        .generate_blocking(
            GenerationRequest {
                stream: false,
                ..GenerationRequest::user_message("FICTIONAL_THIRD_QUESTION")
            },
            Arc::clone(&sink),
        )
        .expect("a ready gateway accepts a blocking generation");
    assert_eq!(outcome.text, "mock blocking answer");
    assert!(!outcome.cancelled);
    assert!(!outcome.saw_reasoning_field);

    // --- stop ---
    let stopped = {
        let gateway = Arc::clone(&gateway);
        within_deadline("stop() on a started gateway", move || gateway.stop())
    };
    let status = stopped.expect("stopping a fake process must succeed");
    assert_eq!(status.state, LocalAiState::Stopped);
    assert!(
        runner.was_killed(),
        "stop must terminate the process the gateway started"
    );
    assert_eq!(
        runner.spawn_count(),
        1,
        "no replacement process may be started"
    );
    assert_eq!(gateway.state(), LocalAiState::Stopped);
    assert!(
        runner.polls.load(Ordering::SeqCst) > 0,
        "try_wait must be used"
    );

    // (b) With no managed server left, a second stop takes the idle branch of
    // `stop()`; it used to ask for a status while still holding the state lock
    // and deadlocked, so it must settle here.
    let second = {
        let gateway = Arc::clone(&gateway);
        within_deadline("a second stop()", move || gateway.stop())
    };
    assert_eq!(second.unwrap().state, LocalAiState::Stopped);
    assert_eq!(gateway.state(), LocalAiState::Stopped);
    assert_eq!(runner.spawn_count(), 1, "stopping must not spawn anything");
}

#[test]
fn an_observed_reasoning_field_survives_a_status_probe() {
    let directory = tempdir().unwrap();
    let (server_path, model_path) = write_fixture_files(directory.path());

    // `/props` can change between probes, and it never reports a reasoning
    // field: only a response can reveal that one exists.
    let advertise_switch = Arc::new(AtomicBool::new(true));
    let responder = {
        let advertise_switch = Arc::clone(&advertise_switch);
        move |request: &MockRequest, stream: &mut TcpStream| match request.path.as_str() {
            HEALTH_PATH => {
                let _ = respond_json(stream, 200, b"{\"status\":\"ok\"}");
            }
            MODELS_PATH => {
                let _ = respond_json(stream, 200, MODELS_WITH_ID.as_bytes());
            }
            PROPS_PATH => {
                let body = if advertise_switch.load(Ordering::SeqCst) {
                    TEMPLATE_WITH_THINKING
                } else {
                    TEMPLATE_ABSENT
                };
                let _ = respond_json(stream, 200, body.as_bytes());
            }
            CHAT_COMPLETIONS_PATH => {
                let _ = respond_sse(stream, &reasoning_stream_pieces());
            }
            _ => {
                let _ = respond_json(stream, 404, b"{}");
            }
        }
    };
    let runner = FakeProcessRunner::new(MockServer::start(responder));
    let port = runner
        .server
        .lock()
        .unwrap()
        .as_ref()
        .expect("the mock server must still be there")
        .port();
    let config = LocalAiConfig {
        server: LocalModelConfig {
            server_path: server_path.to_string_lossy().into_owned(),
            model_path: model_path.to_string_lossy().into_owned(),
            port,
            context_size: 512,
            startup_timeout_seconds: 10,
            ..LocalModelConfig::default()
        },
        ..LocalAiConfig::default()
    };
    let runner_trait: Arc<dyn ProcessRunner> = runner.clone();
    let gateway = Arc::new(LocalAiGateway::with_runner(config, runner_trait));

    let status = gateway
        .start()
        .expect("the fake server answers immediately");
    assert!(
        status.capabilities.thinking_switch,
        "the template advertises the thinking switch"
    );
    assert!(
        !status.capabilities.reasoning_field_observed,
        "no response has been seen yet"
    );

    // One generation whose delta carries reasoning and answer text together.
    let events = Arc::new(Mutex::new(Vec::new()));
    gateway
        .start_generation(
            GenerationRequest::user_message("FICTIONAL_REASONING_QUESTION"),
            event_sink(&events),
        )
        .expect("a ready gateway accepts a generation");
    wait_for(&events, |events| {
        events
            .iter()
            .any(|event| matches!(event, GenerationEvent::Completed { .. }))
    });

    // The worker publishes the observation after the stream ends, and it sets
    // that flag in the same critical section that clears `generating`, so a
    // status query that reports `generating == false` has already seen it.
    let deadline = Instant::now() + EVENT_DEADLINE;
    while gateway.status().unwrap().generating {
        assert!(Instant::now() < deadline, "the generation must settle");
        std::thread::sleep(EVENT_POLL);
    }

    // Every probe from here on reports the same `/props`, which never mentions a
    // reasoning field, so the observation has to survive the refresh.
    for attempt in 1..=2 {
        let status = gateway.status().unwrap();
        assert!(
            status.capabilities.reasoning_field_observed,
            "the observed reasoning field must survive status probe #{attempt}"
        );
        assert!(
            status.capabilities.thinking_switch,
            "the thinking switch must still be what /props said"
        );
        assert_eq!(
            status.capabilities.model_id.as_deref(),
            Some("mock-qwen3-8b")
        );
    }

    // The switch keeps following the probe rather than the observation: a
    // template without the switch takes it away, the observation stays.
    advertise_switch.store(false, Ordering::SeqCst);
    let status = gateway.status().unwrap();
    assert!(
        !status.capabilities.thinking_switch,
        "the probe answer must win for the chat template"
    );
    assert!(
        status.capabilities.reasoning_field_observed,
        "a different template must not erase what a response contained"
    );

    // Leave nothing running: the fake process owns the mock server thread.
    let stopped = {
        let gateway = Arc::clone(&gateway);
        within_deadline("stop() after a reasoning generation", move || {
            gateway.stop()
        })
    };
    assert_eq!(stopped.unwrap().state, LocalAiState::Stopped);
}

#[test]
fn the_gateway_refuses_work_before_a_server_is_running() {
    let gateway = Arc::new(LocalAiGateway::new(LocalAiConfig::default()));
    assert_eq!(gateway.state(), LocalAiState::Stopped);

    // An empty request is refused before anything else is considered.
    assert_eq!(
        gateway
            .start_generation(GenerationRequest::default(), Arc::new(|_| {}))
            .unwrap_err(),
        ChatError::InvalidConfiguration("write a message first".to_string())
    );
    // A real request with no managed server is refused as well.
    assert_eq!(
        gateway
            .start_generation(
                GenerationRequest::user_message("FICTIONAL_USER_QUESTION"),
                Arc::new(|_| {}),
            )
            .unwrap_err(),
        ChatError::ServerNotRunning
    );
    assert!(!gateway.cancel(), "there is nothing to cancel");

    // (a) A gateway that never started stops immediately. This is the regression
    // guard for the idle branch of `stop()`.
    let stopped = {
        let gateway = Arc::clone(&gateway);
        within_deadline("stop() on a gateway with no server", move || gateway.stop())
    };
    assert_eq!(stopped.unwrap().state, LocalAiState::Stopped);
    assert_eq!(gateway.state(), LocalAiState::Stopped);
}

#[test]
fn stopping_shutting_down_and_restarting_an_idle_gateway_never_hangs() {
    // `stop()` used to call `status()` while still holding the non-reentrant
    // state lock, so every stop on a gateway without a managed server deadlocked
    // the caller. Each call here runs under a deadline, so a regression fails the
    // test instead of hanging the binary.
    let gateway = Arc::new(LocalAiGateway::new(LocalAiConfig::default()));

    // (b) Stopping twice in a row, with nothing ever managed, is idempotent.
    for attempt in ["a first stop()", "a second stop()"] {
        let stopped = {
            let gateway = Arc::clone(&gateway);
            within_deadline(attempt, move || gateway.stop())
        };
        assert_eq!(stopped.unwrap().state, LocalAiState::Stopped);
    }

    // (c) `shutdown()` reports nothing and must return as well.
    {
        let gateway = Arc::clone(&gateway);
        within_deadline("shutdown() on an idle gateway", move || gateway.shutdown());
    }
    assert_eq!(gateway.state(), LocalAiState::Stopped);

    // (d) `restart()` stops first and then tries to start. The default
    // configuration names no files, so validation refuses it: what matters is
    // that it comes back with a controlled error instead of blocking forever.
    let restarted = {
        let gateway = Arc::clone(&gateway);
        within_deadline("restart() on an idle gateway", move || gateway.restart())
    };
    let error = restarted.expect_err("an unconfigured gateway cannot start");
    assert!(
        matches!(error, ChatError::InsufficientResources(_)),
        "got {error:?}"
    );
    assert_eq!(gateway.state(), LocalAiState::Failed);

    // A refused start must leave the gateway stoppable again.
    let stopped = {
        let gateway = Arc::clone(&gateway);
        within_deadline("stop() after a refused restart", move || gateway.stop())
    };
    assert_eq!(stopped.unwrap().state, LocalAiState::Stopped);
    assert_eq!(gateway.state(), LocalAiState::Stopped);
}

#[test]
fn the_gateway_refuses_to_start_an_invalid_configuration_without_spawning() {
    let directory = tempdir().unwrap();
    let mock = MockServer::start(|_request: &MockRequest, stream: &mut TcpStream| {
        let _ = respond_json(stream, 200, b"{}");
    });
    let runner = FakeProcessRunner::new(mock);
    let config = LocalAiConfig {
        server: LocalModelConfig {
            server_path: directory
                .path()
                .join("absent.exe")
                .to_string_lossy()
                .into_owned(),
            model_path: directory
                .path()
                .join("absent.gguf")
                .to_string_lossy()
                .into_owned(),
            ..LocalModelConfig::default()
        },
        ..LocalAiConfig::default()
    };
    let runner_trait: Arc<dyn ProcessRunner> = runner.clone();
    let gateway = LocalAiGateway::with_runner(config, runner_trait);

    let error = gateway.start().unwrap_err();

    assert!(
        matches!(error, ChatError::InsufficientResources(_)),
        "a blocked configuration must be refused before anything runs: {error:?}"
    );
    assert_eq!(gateway.state(), LocalAiState::Failed);
    assert_eq!(
        runner.spawn_count(),
        0,
        "nothing may be spawned before validation passes"
    );
    assert!(!runner.was_killed());
}
