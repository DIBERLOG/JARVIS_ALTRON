//! A minimal HTTP/1.1 client for the local OpenAI-compatible endpoint.
//!
//! Why not a full HTTP stack: the only peer is `llama-server` on the loopback
//! interface, so this client deliberately supports exactly what that needs and
//! nothing else:
//!
//! * plain TCP to `127.0.0.1` or `::1` — a non-loopback host is rejected before a
//!   socket is created, so the runtime physically cannot talk to the network;
//! * no TLS, no redirects, no proxy, no cookies, no authentication headers;
//! * `Content-Length`, `Transfer-Encoding: chunked`, and close-delimited bodies;
//! * incremental line reading, which is what streaming (SSE) needs;
//! * a short socket read timeout so a cancellation request is noticed between
//!   tokens instead of only when the server sends the next one.
//!
//! The response head and the chunk framing are read one byte at a time. That keeps
//! the body buffer free of framing bytes, which is what makes the incremental
//! reader correct for chunked streams; the head is a few hundred bytes, so the
//! extra syscalls on loopback are irrelevant next to a token generation.
//!
//! Response bodies are size-bounded and never logged.

use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::ai::ChatError;

/// Maximum size of a non-streaming response body.
pub const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum length of a single line, which bounds SSE event size.
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum length of a response head line.
pub const MAX_HEAD_LINE_BYTES: usize = 16 * 1024;
/// Maximum number of response headers accepted.
pub const MAX_HEADERS: usize = 128;
/// Default connect timeout.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Default socket read timeout: short, so cancellation stays responsive.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_millis(300);
/// Default time without any data after which a stream is considered stalled.
pub const DEFAULT_STALL_TIMEOUT: Duration = Duration::from_secs(90);

const READ_CHUNK_BYTES: usize = 8 * 1024;

/// A loopback endpoint of the local server.
#[derive(Clone, Debug)]
pub struct LoopbackEndpoint {
    address: SocketAddr,
    host_header: String,
    connect_timeout: Duration,
    read_timeout: Duration,
    stall_timeout: Duration,
}

impl LoopbackEndpoint {
    /// Builds an endpoint, refusing anything that is not loopback.
    pub fn new(host: &str, port: u16) -> Result<Self, ChatError> {
        let address = loopback_address(host, port)?;
        Ok(Self {
            address,
            host_header: format!("{host}:{port}"),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            read_timeout: DEFAULT_READ_TIMEOUT,
            stall_timeout: DEFAULT_STALL_TIMEOUT,
        })
    }

    pub fn with_timeouts(
        mut self,
        connect_timeout: Duration,
        read_timeout: Duration,
        stall_timeout: Duration,
    ) -> Self {
        self.connect_timeout = connect_timeout;
        self.read_timeout = read_timeout;
        self.stall_timeout = stall_timeout;
        self
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn is_loopback(&self) -> bool {
        self.address.ip().is_loopback()
    }

    /// Sends a request and returns the parsed head plus a body reader.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        accept: &str,
        body: Option<&str>,
    ) -> Result<HttpResponse<TcpStream>, ChatError> {
        let mut stream = TcpStream::connect_timeout(&self.address, self.connect_timeout)?;
        stream.set_read_timeout(Some(self.read_timeout))?;
        stream.set_write_timeout(Some(self.connect_timeout))?;
        stream.set_nodelay(true).ok();

        let mut request = String::with_capacity(256);
        request.push_str(method);
        request.push(' ');
        request.push_str(path);
        request.push_str(" HTTP/1.1\r\nHost: ");
        request.push_str(&self.host_header);
        request.push_str("\r\nUser-Agent: JARVIS-Altront-local-ai\r\nAccept: ");
        request.push_str(accept);
        request.push_str("\r\nConnection: close\r\n");
        if let Some(body) = body {
            request.push_str("Content-Type: application/json\r\nContent-Length: ");
            request.push_str(&body.len().to_string());
            request.push_str("\r\n");
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes())?;
        if let Some(body) = body {
            stream.write_all(body.as_bytes())?;
        }
        stream.flush()?;

        let mut reader = BodyReader::new(stream, MAX_BODY_BYTES, self.stall_timeout);
        let head = read_response_head(&mut reader)?;
        Ok(HttpResponse { head, body: reader })
    }

    /// Convenience wrapper for a `GET` that returns a whole JSON body.
    pub fn get_json(&self, path: &str) -> Result<(u16, Vec<u8>), ChatError> {
        let response = self.request("GET", path, "application/json", None)?;
        let status = response.head.status;
        let body = response.into_body().read_to_end()?;
        Ok((status, body))
    }

    /// Convenience wrapper for a `POST` that returns a whole JSON body.
    pub fn post_json(&self, path: &str, body: &str) -> Result<(u16, Vec<u8>), ChatError> {
        let response = self.request("POST", path, "application/json", Some(body))?;
        let status = response.head.status;
        let body = response.into_body().read_to_end()?;
        Ok((status, body))
    }
}

/// Resolves a loopback address or refuses the request.
pub fn loopback_address(host: &str, port: u16) -> Result<SocketAddr, ChatError> {
    match host.trim() {
        "127.0.0.1" | "localhost" => Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)),
        "::1" => Ok(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port)),
        "0.0.0.0" | "::" => Err(ChatError::InvalidConfiguration(
            "binding to all interfaces is refused; use 127.0.0.1".to_string(),
        )),
        other => Err(ChatError::InvalidConfiguration(format!(
            "'{other}' is not a loopback host; the local AI talks only to 127.0.0.1"
        ))),
    }
}

/// Parsed response head.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResponseHead {
    pub status: u16,
    /// Lower-cased header names.
    pub headers: Vec<(String, String)>,
}

impl ResponseHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A response with a body reader that can be consumed incrementally.
#[derive(Debug)]
pub struct HttpResponse<R: Read> {
    pub head: ResponseHead,
    body: BodyReader<R>,
}

impl<R: Read> HttpResponse<R> {
    pub fn into_body(self) -> BodyReader<R> {
        self.body
    }

    pub fn body_mut(&mut self) -> &mut BodyReader<R> {
        &mut self.body
    }
}

enum BodyMode {
    /// Exact number of bytes still to read.
    Length(u64),
    Chunked {
        remaining_in_chunk: u64,
        finished: bool,
    },
    /// Until the peer closes the connection.
    UntilClose,
}

enum FillOutcome {
    /// New body bytes are buffered.
    Data,
    /// A read timed out with no data; the caller decides whether to keep waiting.
    Idle,
    Eof,
}

/// Incremental body reader with line access.
///
/// Its buffer holds **body bytes only**: framing is consumed by byte-level reads
/// before any bulk read, and a `Content-Length` body never reads past its length.
pub struct BodyReader<R: Read> {
    source: R,
    buffer: VecDeque<u8>,
    mode: BodyMode,
    max_bytes: u64,
    produced: u64,
    stall_timeout: Duration,
    last_data_at: Instant,
    finished: bool,
}

impl<R: Read> std::fmt::Debug for BodyReader<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BodyReader")
            .field("produced", &self.produced)
            .field("finished", &self.finished)
            .finish()
    }
}

impl<R: Read> BodyReader<R> {
    /// Builds a reader that assumes the whole stream is the body (tests).
    pub fn unbounded(source: R, stall_timeout: Duration) -> Self {
        Self::new_with_mode(source, BodyMode::UntilClose, MAX_BODY_BYTES, stall_timeout)
    }

    fn new(source: R, max_bytes: u64, stall_timeout: Duration) -> Self {
        Self::new_with_mode(source, BodyMode::UntilClose, max_bytes, stall_timeout)
    }

    fn new_with_mode(source: R, mode: BodyMode, max_bytes: u64, stall_timeout: Duration) -> Self {
        Self {
            source,
            buffer: VecDeque::new(),
            mode,
            max_bytes,
            produced: 0,
            stall_timeout,
            last_data_at: Instant::now(),
            finished: false,
        }
    }

    /// Reads the whole body, bounded by the configured maximum.
    pub fn read_to_end(&mut self) -> Result<Vec<u8>, ChatError> {
        let mut output = Vec::new();
        loop {
            if !self.buffer.is_empty() {
                output.extend(self.buffer.drain(..));
                if output.len() as u64 > self.max_bytes {
                    return Err(ChatError::InvalidStream);
                }
                continue;
            }
            match self.fill()? {
                FillOutcome::Data => continue,
                FillOutcome::Idle => continue,
                FillOutcome::Eof => {
                    output.extend(self.buffer.drain(..));
                    return Ok(output);
                }
            }
        }
    }

    /// Returns the next line, without its terminator.
    ///
    /// `Ok(None)` means the body ended. A cancellation flag set by another thread
    /// is noticed while waiting for data.
    pub fn next_line(&mut self, cancel: &AtomicBool) -> Result<Option<String>, ChatError> {
        loop {
            if let Some(line) = self.take_line()? {
                return Ok(Some(line));
            }
            if self.finished {
                return Ok(None);
            }
            match self.fill()? {
                FillOutcome::Data => continue,
                FillOutcome::Idle => {
                    if cancel.load(Ordering::Relaxed) {
                        return Err(ChatError::Cancelled);
                    }
                }
                FillOutcome::Eof => {
                    // A final line without a terminator is still a line.
                    if let Some(line) = self.take_final_line()? {
                        return Ok(Some(line));
                    }
                    return Ok(None);
                }
            }
        }
    }

    /// Whether the body has been fully consumed.
    pub fn is_finished(&self) -> bool {
        self.finished && self.buffer.is_empty()
    }

    fn take_line(&mut self) -> Result<Option<String>, ChatError> {
        let newline = self.buffer.iter().position(|byte| *byte == b'\n');
        match newline {
            None => {
                if self.buffer.len() > MAX_LINE_BYTES {
                    return Err(ChatError::InvalidStream);
                }
                Ok(None)
            }
            Some(position) => {
                if position > MAX_LINE_BYTES {
                    return Err(ChatError::InvalidStream);
                }
                let bytes: Vec<u8> = self.buffer.drain(..=position).collect();
                Ok(Some(decode_line(&bytes[..position])?))
            }
        }
    }

    /// Returns whatever is left after the body ended, if anything.
    fn take_final_line(&mut self) -> Result<Option<String>, ChatError> {
        if self.buffer.is_empty() {
            return Ok(None);
        }
        let bytes: Vec<u8> = self.buffer.drain(..).collect();
        if bytes.is_empty() {
            return Ok(None);
        }
        Ok(Some(decode_line(&bytes)?))
    }

    fn fill(&mut self) -> Result<FillOutcome, ChatError> {
        if self.finished {
            return Ok(FillOutcome::Eof);
        }
        match &self.mode {
            BodyMode::Length(0) => {
                self.finished = true;
                return Ok(FillOutcome::Eof);
            }
            BodyMode::Chunked { finished: true, .. } => {
                self.finished = true;
                return Ok(FillOutcome::Eof);
            }
            _ => {}
        }

        // A chunk is only read once its size is known, so framing never mixes
        // with content.
        if let BodyMode::Chunked {
            remaining_in_chunk, ..
        } = self.mode
        {
            if remaining_in_chunk == 0 {
                match self.read_chunk_header()? {
                    Some(0) => {
                        self.mode = BodyMode::Chunked {
                            remaining_in_chunk: 0,
                            finished: true,
                        };
                        self.finished = true;
                        return Ok(FillOutcome::Eof);
                    }
                    Some(size) => {
                        self.mode = BodyMode::Chunked {
                            remaining_in_chunk: size,
                            finished: false,
                        };
                    }
                    None => {
                        self.finished = true;
                        return Ok(FillOutcome::Eof);
                    }
                }
            }
        }

        let limit = match self.mode {
            BodyMode::Length(remaining) => remaining.min(READ_CHUNK_BYTES as u64) as usize,
            BodyMode::Chunked {
                remaining_in_chunk, ..
            } => (remaining_in_chunk as usize).min(READ_CHUNK_BYTES),
            BodyMode::UntilClose => READ_CHUNK_BYTES,
        };
        if limit == 0 {
            self.finished = true;
            return Ok(FillOutcome::Eof);
        }

        let mut chunk = vec![0u8; limit];
        match self.source.read(&mut chunk) {
            Ok(0) => {
                self.finished = true;
                Ok(FillOutcome::Eof)
            }
            Ok(read) => {
                chunk.truncate(read);
                self.produced += read as u64;
                self.buffer.extend(chunk);
                self.last_data_at = Instant::now();
                match &mut self.mode {
                    BodyMode::Length(remaining) => {
                        *remaining = remaining.saturating_sub(read as u64)
                    }
                    BodyMode::Chunked {
                        remaining_in_chunk, ..
                    } => {
                        *remaining_in_chunk = remaining_in_chunk.saturating_sub(read as u64);
                        if *remaining_in_chunk == 0 {
                            self.consume_chunk_terminator()?;
                        }
                    }
                    BodyMode::UntilClose => {}
                }
                Ok(FillOutcome::Data)
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if self.last_data_at.elapsed() >= self.stall_timeout {
                    Err(ChatError::TimedOut)
                } else {
                    Ok(FillOutcome::Idle)
                }
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Reads one byte that belongs to framing, never to the body.
    fn read_framing_byte(&mut self) -> Result<Option<u8>, ChatError> {
        let mut byte = [0u8; 1];
        loop {
            match self.source.read(&mut byte) {
                Ok(0) => return Ok(None),
                Ok(_) => {
                    self.last_data_at = Instant::now();
                    return Ok(Some(byte[0]));
                }
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    if self.last_data_at.elapsed() >= self.stall_timeout {
                        return Err(ChatError::TimedOut);
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Reads a whole line byte by byte, for the head and for chunk sizes.
    fn read_framing_line(&mut self, max_bytes: usize) -> Result<Option<Vec<u8>>, ChatError> {
        let mut line = Vec::new();
        loop {
            let Some(byte) = self.read_framing_byte()? else {
                return if line.is_empty() {
                    Ok(None)
                } else {
                    Err(ChatError::InvalidStream)
                };
            };
            if byte == b'\n' {
                return Ok(Some(line));
            }
            if byte != b'\r' {
                line.push(byte);
            }
            if line.len() > max_bytes {
                return Err(ChatError::InvalidStream);
            }
        }
    }

    /// Reads a chunk-size line.
    fn read_chunk_header(&mut self) -> Result<Option<u64>, ChatError> {
        let Some(line) = self.read_framing_line(128)? else {
            return Ok(None);
        };
        let text = String::from_utf8(line).map_err(|_| ChatError::InvalidStream)?;
        // Chunk extensions after ';' are ignored.
        let size_text = text.split(';').next().unwrap_or("").trim();
        u64::from_str_radix(size_text, 16)
            .map(Some)
            .map_err(|_| ChatError::InvalidStream)
    }

    /// Consumes the CRLF that terminates a chunk.
    fn consume_chunk_terminator(&mut self) -> Result<(), ChatError> {
        for _ in 0..2 {
            let Some(byte) = self.read_framing_byte()? else {
                return Ok(());
            };
            if byte == b'\n' {
                return Ok(());
            }
        }
        Ok(())
    }
}

/// Parses the status line and headers from a body reader.
///
/// The head is read byte by byte, so nothing beyond it enters the body buffer.
pub fn read_response_head<R: Read>(reader: &mut BodyReader<R>) -> Result<ResponseHead, ChatError> {
    let status_line = reader
        .read_framing_line(MAX_HEAD_LINE_BYTES)?
        .ok_or(ChatError::InvalidStream)?;
    let status_line = String::from_utf8(status_line).map_err(|_| ChatError::InvalidStream)?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/1.") {
        return Err(ChatError::InvalidStream);
    }
    let status: u16 = parts
        .next()
        .and_then(|value| value.trim().parse().ok())
        .ok_or(ChatError::InvalidStream)?;
    if !(200..600).contains(&status) {
        // Informational responses are not expected from this endpoint.
        return Err(ChatError::InvalidStream);
    }

    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let line = reader
            .read_framing_line(MAX_HEAD_LINE_BYTES)?
            .ok_or(ChatError::InvalidStream)?;
        if line.is_empty() {
            break;
        }
        let line = String::from_utf8(line).map_err(|_| ChatError::InvalidStream)?;
        // Obsolete line folding: a continuation line starts with whitespace.
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, value)) = headers.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
        if headers.len() > MAX_HEADERS {
            return Err(ChatError::InvalidStream);
        }
    }

    // The body mode follows from the headers, in the order HTTP requires.
    let mode = if headers
        .iter()
        .find(|(name, _)| name == "transfer-encoding")
        .map(|(_, value)| value.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
    {
        BodyMode::Chunked {
            remaining_in_chunk: 0,
            finished: false,
        }
    } else if let Some(length) = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.trim().parse::<u64>().ok())
    {
        BodyMode::Length(length)
    } else {
        BodyMode::UntilClose
    };
    reader.mode = mode;
    Ok(ResponseHead { status, headers })
}

fn decode_line(bytes: &[u8]) -> Result<String, ChatError> {
    let mut end = bytes.len();
    if end > 0 && bytes[end - 1] == b'\r' {
        end -= 1;
    }
    // Invalid UTF-8 is refused rather than guessed, because callers treat lines
    // as text.
    String::from_utf8(bytes[..end].to_vec()).map_err(|_| ChatError::InvalidStream)
}

/// A helper used by tests.
pub fn header_list(headers: &[(&str, &str)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn reader_from(bytes: &[u8]) -> BodyReader<Cursor<Vec<u8>>> {
        BodyReader::unbounded(Cursor::new(bytes.to_vec()), DEFAULT_STALL_TIMEOUT)
    }

    fn head_and_body(response: &str) -> (ResponseHead, BodyReader<Cursor<Vec<u8>>>) {
        let mut reader = reader_from(response.as_bytes());
        let head = read_response_head(&mut reader).unwrap();
        (head, reader)
    }

    #[test]
    fn only_loopback_hosts_are_accepted() {
        assert!(loopback_address("127.0.0.1", 8080)
            .unwrap()
            .ip()
            .is_loopback());
        assert!(loopback_address("localhost", 8080)
            .unwrap()
            .ip()
            .is_loopback());
        assert!(loopback_address("::1", 8080).unwrap().ip().is_loopback());
        for rejected in ["0.0.0.0", "::", "192.168.1.5", "example.com", "8.8.8.8", ""] {
            assert!(
                loopback_address(rejected, 8080).is_err(),
                "{rejected} must be refused"
            );
        }
        let endpoint = LoopbackEndpoint::new("127.0.0.1", 9090).unwrap();
        assert!(endpoint.is_loopback());
        assert_eq!(endpoint.address().port(), 9090);
        assert!(LoopbackEndpoint::new("10.0.0.1", 9090).is_err());
    }

    #[test]
    fn parses_a_status_line_and_headers_case_insensitively() {
        let (head, _) = head_and_body(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Test: a\r\n\r\n{}",
        );
        assert_eq!(head.status, 200);
        assert!(head.is_success());
        assert_eq!(head.header("content-type"), Some("application/json"));
        assert_eq!(head.header("X-TEST"), Some("a"));
        assert_eq!(head.header("absent"), None);
    }

    #[test]
    fn reads_a_content_length_body_and_stops_there() {
        let (head, mut body) =
            head_and_body("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhelloEXTRA");
        assert_eq!(head.status, 200);
        // The declared length wins: trailing bytes are not part of the body.
        assert_eq!(body.read_to_end().unwrap(), b"hello");
    }

    #[test]
    fn decodes_a_chunked_body_including_extensions() {
        let response = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
            5\r\nhello\r\n6;ext=1\r\n world\r\n0\r\n\r\n";
        let (head, mut body) = head_and_body(response);
        assert_eq!(head.status, 200);
        assert_eq!(body.read_to_end().unwrap(), b"hello world");
    }

    #[test]
    fn decodes_a_chunked_body_split_across_reads() {
        // A chunked stream arriving in small pieces must still decode.
        let pieces: Vec<Vec<u8>> = vec![
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec(),
            b"3\r\nab".to_vec(),
            b"c\r\n2\r\nde\r\n0\r\n\r\n".to_vec(),
        ];
        struct Piecewise {
            pieces: Vec<Vec<u8>>,
            index: usize,
            offset: usize,
        }
        impl Read for Piecewise {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                while self.index < self.pieces.len() {
                    let piece = &self.pieces[self.index];
                    if self.offset < piece.len() {
                        let available = (piece.len() - self.offset).min(buffer.len());
                        buffer[..available]
                            .copy_from_slice(&piece[self.offset..self.offset + available]);
                        self.offset += available;
                        return Ok(available);
                    }
                    self.index += 1;
                    self.offset = 0;
                }
                Ok(0)
            }
        }
        let mut reader = BodyReader::unbounded(
            Piecewise {
                pieces,
                index: 0,
                offset: 0,
            },
            DEFAULT_STALL_TIMEOUT,
        );
        let head = read_response_head(&mut reader).unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(reader.read_to_end().unwrap(), b"abcde");
    }

    #[test]
    fn reads_a_close_delimited_body() {
        let (head, mut body) = head_and_body("HTTP/1.1 200 OK\r\n\r\nuntil close");
        assert_eq!(head.status, 200);
        assert_eq!(body.read_to_end().unwrap(), b"until close");
    }

    #[test]
    fn reads_lines_incrementally_with_unicode_and_empty_lines() {
        let payload = "data: привет\n\ndata: ok\n";
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{}",
            payload.len(),
            payload
        );
        let (_, mut body) = head_and_body(&response);
        let cancel = AtomicBool::new(false);
        assert_eq!(body.next_line(&cancel).unwrap().unwrap(), "data: привет");
        assert_eq!(body.next_line(&cancel).unwrap().unwrap(), "");
        assert_eq!(body.next_line(&cancel).unwrap().unwrap(), "data: ok");
        assert_eq!(body.next_line(&cancel).unwrap(), None);
    }

    #[test]
    fn a_last_line_without_a_newline_is_not_lost() {
        let mut body = reader_from(b"first\nsecond");
        let cancel = AtomicBool::new(false);
        assert_eq!(body.next_line(&cancel).unwrap().unwrap(), "first");
        assert_eq!(body.next_line(&cancel).unwrap().unwrap(), "second");
        assert_eq!(body.next_line(&cancel).unwrap(), None);
    }

    #[test]
    fn incomplete_and_malformed_responses_are_refused() {
        let mut reader = reader_from(b"NOT-HTTP nonsense\r\n\r\n");
        assert_eq!(
            read_response_head(&mut reader).unwrap_err(),
            ChatError::InvalidStream
        );

        let mut reader = reader_from(b"HTTP/1.1 abc\r\n\r\n");
        assert_eq!(
            read_response_head(&mut reader).unwrap_err(),
            ChatError::InvalidStream
        );

        // Headers without a terminating empty line.
        let mut reader = reader_from(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n");
        assert_eq!(
            read_response_head(&mut reader).unwrap_err(),
            ChatError::InvalidStream
        );

        // A chunk size that is not hexadecimal.
        let response = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nbroken\r\n";
        let (_, mut body) = head_and_body(response);
        assert_eq!(body.read_to_end().unwrap_err(), ChatError::InvalidStream);

        // Invalid UTF-8 inside a line.
        let mut reader = reader_from(&[b'a', 0xff, b'\n']);
        let cancel = AtomicBool::new(false);
        assert_eq!(
            reader.next_line(&cancel).unwrap_err(),
            ChatError::InvalidStream
        );
    }

    #[test]
    fn an_over_long_body_is_refused_by_the_size_bound() {
        let mut reader =
            BodyReader::new(Cursor::new(vec![b'x'; 4096]), 1024, DEFAULT_STALL_TIMEOUT);
        assert_eq!(reader.read_to_end().unwrap_err(), ChatError::InvalidStream);
    }

    #[test]
    fn an_over_long_line_is_refused() {
        let mut bytes = vec![b'x'; MAX_LINE_BYTES + 2];
        bytes.push(b'\n');
        let mut reader = BodyReader::unbounded(Cursor::new(bytes), DEFAULT_STALL_TIMEOUT);
        let cancel = AtomicBool::new(false);
        assert_eq!(
            reader.next_line(&cancel).unwrap_err(),
            ChatError::InvalidStream
        );
    }

    #[test]
    fn cancellation_is_reported_while_waiting_for_data() {
        // An endless source that immediately reports a timeout: the reader must
        // return Cancelled instead of spinning forever.
        struct Idle;
        impl Read for Idle {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(ErrorKind::WouldBlock, "idle"))
            }
        }
        let mut reader = BodyReader::unbounded(Idle, Duration::from_secs(60));
        let cancel = AtomicBool::new(true);
        assert_eq!(reader.next_line(&cancel).unwrap_err(), ChatError::Cancelled);
    }

    #[test]
    fn a_stalled_stream_times_out() {
        struct Idle;
        impl Read for Idle {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(ErrorKind::WouldBlock, "idle"))
            }
        }
        let mut reader = BodyReader::unbounded(Idle, Duration::ZERO);
        let cancel = AtomicBool::new(false);
        assert_eq!(reader.next_line(&cancel).unwrap_err(), ChatError::TimedOut);
        let mut reader = BodyReader::unbounded(Idle, Duration::ZERO);
        assert_eq!(reader.read_to_end().unwrap_err(), ChatError::TimedOut);
    }

    #[test]
    fn folded_headers_and_extra_whitespace_are_tolerated() {
        let head = head_and_body(
            "HTTP/1.1 200 OK\r\nContent-Type:   application/json  \r\nX-Long: one\r\n  two\r\n\r\n",
        )
        .0;
        assert_eq!(head.header("content-type"), Some("application/json"));
        assert_eq!(head.header("x-long"), Some("one two"));
        assert_eq!(head.header("content-length"), None);
    }

    #[test]
    fn informational_statuses_and_bad_statuses_are_refused() {
        let mut reader = reader_from(b"HTTP/1.1 100 Continue\r\n\r\n");
        assert_eq!(
            read_response_head(&mut reader).unwrap_err(),
            ChatError::InvalidStream
        );
        let mut reader = reader_from(b"HTTP/1.1 999 Weird\r\n\r\n");
        assert_eq!(
            read_response_head(&mut reader).unwrap_err(),
            ChatError::InvalidStream
        );
    }

    #[test]
    fn a_missing_content_length_still_parses_the_head() {
        let (head, mut body) = head_and_body("HTTP/1.1 200 OK\r\nX: 1\r\n\r\nbody");
        assert_eq!(head.header("x"), Some("1"));
        assert_eq!(body.read_to_end().unwrap(), b"body");
    }

    #[test]
    fn an_empty_body_is_read_as_nothing() {
        let (head, mut body) =
            head_and_body("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(head.status, 204);
        assert_eq!(body.read_to_end().unwrap(), b"");
    }

    #[test]
    fn the_header_helper_lowercases_names() {
        let headers = header_list(&[("Content-Type", "application/json")]);
        assert_eq!(headers[0].0, "content-type");
    }
}
