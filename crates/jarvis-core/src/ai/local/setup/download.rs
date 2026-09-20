//! Resumable, verified download of a pinned artifact.
//!
//! The rules this module implements, in the order they protect the user:
//!
//! * only a compiled-in artifact may be fetched — a URL that is not one of the
//!   pinned manifests is refused before a socket is opened, so neither the
//!   interface nor a model response can redirect an installation;
//! * the body is written to `<name>.part` and only becomes `<name>` after its
//!   length and its SHA-256 have both been verified, so a partial file can never
//!   be mistaken for a complete one;
//! * `Content-Length` is checked when the server sends it, and a hard byte cap is
//!   enforced while reading, so a lying or missing header cannot fill the disk;
//! * a resume is only attempted when the server *proves* it supports ranges (a
//!   `206` with a matching `Content-Range`) **and** the stored `ETag` or
//!   `Last-Modified` still matches. A server that answers `200` to a ranged
//!   request is never appended to: the file is truncated and restarted;
//! * the number of attempts is bounded and a stalled attempt counts against it,
//!   so a broken endpoint cannot produce an endless retry loop;
//! * cancellation is checked for every chunk. A cancelled download keeps its
//!   `.part` file — that is what makes the next run resumable — while a download
//!   that failed a *correctness* check deletes it, because those bytes are known
//!   to be wrong.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::super::managed::{
    managed_model_manifest, managed_runtime_manifest, DownloadProgress,
};

/// How many times one download may be attempted before it gives up.
pub const MAX_DOWNLOAD_ATTEMPTS: u32 = 3;
/// Time allowed for the connection to be established.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// Time allowed between two reads of the body.
pub const READ_TIMEOUT: Duration = Duration::from_secs(45);
/// Read buffer size.
pub const CHUNK_BYTES: usize = 64 * 1024;
/// Consecutive attempts that transfer nothing before the download gives up.
pub const MAX_STALLED_ATTEMPTS: u32 = 2;
/// Longest file name this module will write.
const MAX_FILENAME_CHARS: usize = 200;

/// A pinned artifact, as the backend describes it. It never leaves the backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactExpectation {
    /// Pinned HTTPS location. Compiled in, never supplied by the interface.
    pub url: String,
    pub filename: String,
    pub expected_size: u64,
    /// Lowercase hexadecimal SHA-256 of the complete file.
    pub sha256: String,
}

impl From<super::super::managed::ManagedArtifact> for ArtifactExpectation {
    fn from(artifact: super::super::managed::ManagedArtifact) -> Self {
        Self {
            url: artifact.source_url.to_string(),
            filename: artifact.filename.to_string(),
            expected_size: artifact.expected_size,
            sha256: artifact.sha256.to_string(),
        }
    }
}

/// How much the caller is allowed to trust an artifact description.
///
/// The application only ever builds [`ArtifactTrust::Pinned`]. The loopback
/// variant exists so the tests can drive this module against a local server with
/// tiny fixtures, and it refuses anything that is not a loopback address, so a
/// mistake in a test cannot become a way to fetch from the internet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactTrust {
    /// The artifact must be one of the manifests compiled into this build.
    Pinned,
    /// A loopback address, for tests with a local fake server.
    LoopbackTesting,
}

/// Why a download did not complete.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownloadError {
    /// The URL is not one of the pinned manifests, or is not HTTPS.
    RefusedUrl,
    /// The destination file name is not the artifact's own name.
    Destination,
    /// The body was longer than the pinned size, whatever the headers said.
    TooLarge,
    /// The completed body had the wrong length.
    SizeMismatch,
    /// The completed body had the wrong SHA-256.
    HashMismatch,
    /// The server's `Content-Length` disagreed with the pinned size.
    ContentLengthMismatch,
    /// A ranged response did not describe the range that was requested.
    RangeMismatch,
    /// The resource behind the URL is not the one that was partially fetched.
    IdentityChanged,
    Cancelled,
    /// The connection failed, or the body ended early.
    Network,
    Timeout,
    Io,
    /// Every attempt failed without making progress.
    NoProgress,
}

impl DownloadError {
    /// Whether trying again could plausibly succeed.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Network | Self::Timeout)
    }

    /// Short, stable code for the interface.
    pub fn code(&self) -> &'static str {
        match self {
            Self::RefusedUrl => "refused_url",
            Self::Destination => "bad_destination",
            Self::TooLarge => "too_large",
            Self::SizeMismatch => "size_mismatch",
            Self::HashMismatch => "hash_mismatch",
            Self::ContentLengthMismatch => "content_length_mismatch",
            Self::RangeMismatch => "range_mismatch",
            Self::IdentityChanged => "identity_changed",
            Self::Cancelled => "cancelled",
            Self::Network => "network",
            Self::Timeout => "timeout",
            Self::Io => "io",
            Self::NoProgress => "no_progress",
        }
    }
}

/// What a download produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DownloadOutcome {
    pub bytes: u64,
    /// Bytes that were already on the disk when this call started.
    pub resumed_from: u64,
    /// How many requests were made.
    pub fetches: u32,
}

/// One byte range the server reported.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContentRange {
    pub start: u64,
    pub end: u64,
    pub total: Option<u64>,
}

/// Parses `Content-Range: bytes <start>-<end>/<total|*>`.
pub fn parse_content_range(value: &str) -> Option<ContentRange> {
    let value = value.trim();
    let rest = value.strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start = start.trim().parse::<u64>().ok()?;
    let end = end.trim().parse::<u64>().ok()?;
    let total = match total.trim() {
        "*" => None,
        other => Some(other.parse::<u64>().ok()?),
    };
    if end < start {
        return None;
    }
    Some(ContentRange { start, end, total })
}

/// A response head plus its body, as the downloader needs it.
pub struct TransportResponse {
    pub status: u16,
    /// `Content-Length` of this response, when the server sent one.
    pub content_length: Option<u64>,
    pub content_range: Option<ContentRange>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub accept_ranges: bool,
    pub body: Box<dyn Read + Send>,
}

/// The HTTP transport, so the downloader can be exercised without a network.
pub trait DownloadTransport: Send + Sync {
    /// Sends one GET, optionally asking for the tail starting at `range_from`.
    fn fetch(&self, url: &str, range_from: Option<u64>) -> Result<TransportResponse, DownloadError>;
}

/// The production transport.
pub struct ReqwestTransport {
    client: reqwest::blocking::Client,
}

impl ReqwestTransport {
    pub fn new() -> Result<Self, DownloadError> {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(READ_TIMEOUT)
            .build()
            .map_err(|_| DownloadError::Network)?;
        Ok(Self { client })
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new().expect("the HTTP client can always be built")
    }
}

fn map_transport_error(error: reqwest::Error) -> DownloadError {
    if error.is_timeout() {
        DownloadError::Timeout
    } else {
        DownloadError::Network
    }
}

impl DownloadTransport for ReqwestTransport {
    fn fetch(&self, url: &str, range_from: Option<u64>) -> Result<TransportResponse, DownloadError> {
        let mut request = self.client.get(url);
        if let Some(from) = range_from {
            request = request.header(reqwest::header::RANGE, format!("bytes={from}-"));
        }
        let response = request.send().map_err(map_transport_error)?;
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let header = |name: reqwest::header::HeaderName| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.trim().to_string())
        };
        let content_range =
            header(reqwest::header::CONTENT_RANGE).and_then(|value| parse_content_range(&value));
        let accept_ranges = header(reqwest::header::ACCEPT_RANGES)
            .is_some_and(|value| value.to_ascii_lowercase().contains("bytes"));
        let content_length = response.content_length();
        Ok(TransportResponse {
            status,
            content_length,
            content_range,
            etag: header(reqwest::header::ETAG),
            last_modified: header(reqwest::header::LAST_MODIFIED),
            accept_ranges,
            body: Box::new(response),
        })
    }
}

/// Identity of the resource a `.part` file was fetched from.
///
/// Stored next to the partial file so a later run can prove that the server is
/// still serving the same bytes before appending to them.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PartIdentity {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub expected_size: u64,
    pub sha256: String,
}

/// Path of the identity file that belongs to `part`.
pub fn identity_path(part: &Path) -> PathBuf {
    let mut name = part.file_name().unwrap_or_default().to_os_string();
    name.push(".meta.json");
    part.with_file_name(name)
}

fn read_identity(part: &Path) -> Option<PartIdentity> {
    let text = fs::read_to_string(identity_path(part)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_identity(part: &Path, identity: &PartIdentity) -> Result<(), DownloadError> {
    crate::fsutil::write_json_atomic(&identity_path(part), identity).map_err(|_| DownloadError::Io)
}

/// Removes the partial file *and* its identity: the bytes cannot be trusted.
fn clear_part(part: &Path) {
    let _ = fs::remove_file(part);
    let _ = fs::remove_file(identity_path(part));
}

/// Removes only the identity file, which keeps a verified `.part` in place.
fn clear_identity(part: &Path) {
    let _ = fs::remove_file(identity_path(part));
}

/// Whether an artifact description is one this build is allowed to fetch.
pub fn is_pinned(expectation: &ArtifactExpectation) -> bool {
    [managed_runtime_manifest().artifact, managed_model_manifest().artifact]
        .iter()
        .any(|artifact| {
            artifact.source_url == expectation.url
                && artifact.sha256 == expectation.sha256
                && artifact.expected_size == expectation.expected_size
                && artifact.filename == expectation.filename
        })
}

fn is_loopback_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let host = authority.rsplit_once(':').map_or(authority, |(host, _)| host);
    matches!(host, "127.0.0.1" | "localhost" | "[::1]")
}

fn validate_url(url: &str, trust: ArtifactTrust) -> Result<(), DownloadError> {
    match trust {
        ArtifactTrust::Pinned => {
            if !url.starts_with("https://") {
                return Err(DownloadError::RefusedUrl);
            }
        }
        ArtifactTrust::LoopbackTesting => {
            if !is_loopback_url(url) {
                return Err(DownloadError::RefusedUrl);
            }
        }
    }
    Ok(())
}

/// Validates an artifact description before any network use.
pub fn validate_expectation(
    expectation: &ArtifactExpectation,
    trust: ArtifactTrust,
) -> Result<(), DownloadError> {
    if expectation.filename.is_empty()
        || expectation.filename.contains(['/', '\\', ':'])
        || expectation.filename.chars().count() > MAX_FILENAME_CHARS
        || expectation.expected_size == 0
        || expectation.sha256.len() != 64
        || !expectation.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DownloadError::RefusedUrl);
    }
    validate_url(&expectation.url, trust)?;
    if trust == ArtifactTrust::Pinned && !is_pinned(expectation) {
        return Err(DownloadError::RefusedUrl);
    }
    Ok(())
}

/// SHA-256 of a file, read in bounded chunks so a large model cannot exhaust
/// memory.
pub fn sha256_file(path: &Path) -> Result<String, DownloadError> {
    let mut file = File::open(path).map_err(|_| DownloadError::Io)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    loop {
        let count = file.read(&mut buffer).map_err(|_| DownloadError::Io)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

/// Where a download writes, and what it expects to find.
#[derive(Clone, Debug)]
pub struct DownloadRequest<'a> {
    pub expectation: &'a ArtifactExpectation,
    pub part: &'a Path,
    pub trust: ArtifactTrust,
}

/// Downloads an artifact into its `.part` file and verifies it.
///
/// On `Ok`, the `.part` file holds the complete, hash-verified body. Promoting it
/// to its final name is the caller's step, so nothing is activated before every
/// check has passed.
pub fn download_artifact<F, C>(
    transport: &dyn DownloadTransport,
    request: DownloadRequest<'_>,
    mut progress: F,
    mut cancelled: C,
) -> Result<DownloadOutcome, DownloadError>
where
    F: FnMut(DownloadProgress),
    C: FnMut() -> bool,
{
    let expectation = request.expectation;
    validate_expectation(expectation, request.trust)?;
    let expected_part_name = format!("{}.part", expectation.filename);
    if request.part.file_name().and_then(|name| name.to_str()) != Some(expected_part_name.as_str()) {
        return Err(DownloadError::Destination);
    }
    let part = request.part;
    if let Some(parent) = part.parent() {
        fs::create_dir_all(parent).map_err(|_| DownloadError::Io)?;
    }

    // A `.part` that is already the full size can be settled by hashing it,
    // instead of re-downloading several gigabytes after a crash in the last
    // second of a transfer.
    let existing = part.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    if existing == expectation.expected_size {
        let actual = sha256_file(part)?;
        if actual.eq_ignore_ascii_case(&expectation.sha256) {
            progress(DownloadProgress {
                downloaded: existing,
                total: expectation.expected_size,
            });
            // The verified `.part` stays: the caller promotes it.
            clear_identity(part);
            return Ok(DownloadOutcome {
                bytes: existing,
                resumed_from: existing,
                fetches: 0,
            });
        }
        clear_part(part);
    } else if existing > expectation.expected_size {
        // Longer than the resource it claims to be: unusable.
        clear_part(part);
    }

    let mut attempts = 0_u32;
    let mut stalled = 0_u32;
    let mut last_error: Option<DownloadError> = None;

    loop {
        if attempts >= MAX_DOWNLOAD_ATTEMPTS {
            return Err(last_error.unwrap_or(DownloadError::NoProgress));
        }
        if stalled >= MAX_STALLED_ATTEMPTS {
            return Err(DownloadError::NoProgress);
        }
        if cancelled() {
            // The `.part` file stays: a cancelled download is resumable.
            return Err(DownloadError::Cancelled);
        }
        attempts += 1;

        // Recomputed every attempt, so a body that ended early is resumed from
        // exactly what reached the disk.
        let on_disk = part.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        let identity = read_identity(part);
        let resume = on_disk > 0
            && on_disk < expectation.expected_size
            && identity
                .as_ref()
                .is_some_and(|identity| identity.sha256.eq_ignore_ascii_case(&expectation.sha256));
        let requested_range = if resume { Some(on_disk) } else { None };

        let response = match transport.fetch(&expectation.url, requested_range) {
            Ok(response) => response,
            Err(error) => {
                if !error.is_retryable() {
                    return Err(error);
                }
                last_error = Some(error);
                stalled += 1;
                continue;
            }
        };

        // A `416` means the range is past the end of the resource. Either the
        // file is already complete on disk (handled below), or the resource
        // shrank, in which case the partial file is worthless.
        if response.status == 416 {
            let on_disk = part.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            if on_disk == expectation.expected_size {
                let actual = sha256_file(part)?;
                if actual.eq_ignore_ascii_case(&expectation.sha256) {
                    clear_identity(part);
                    return Ok(DownloadOutcome {
                        bytes: on_disk,
                        resumed_from: on_disk,
                        fetches: attempts,
                    });
                }
            }
            clear_part(part);
            last_error = Some(DownloadError::RangeMismatch);
            stalled += 1;
            continue;
        }
        if response.status != 200 && response.status != 206 {
            // A refusal by the server is not a stall: it is a definite answer,
            // so it burns an attempt but does not trip the progress guard.
            last_error = Some(DownloadError::Network);
            continue;
        }

        // A `200` to a ranged request means the server ignored the range. The
        // answer is a whole body from byte zero, so the partial file must be
        // truncated — appending it would corrupt the result.
        let effective_resume = if response.status == 206 && requested_range.is_some() {
            match response.content_range {
                Some(range)
                    if range.start == on_disk
                        && range.total.is_none_or(|total| total == expectation.expected_size) =>
                {
                    on_disk
                }
                _ => {
                    // No usable proof that the range is the one we asked for.
                    clear_part(part);
                    last_error = Some(DownloadError::RangeMismatch);
                    stalled += 1;
                    continue;
                }
            }
        } else {
            0
        };
        if response.status == 206 && requested_range.is_none() {
            // A partial answer to a full request; only usable if it starts at the
            // beginning, otherwise the file would have a hole.
            match response.content_range {
                Some(range) if range.start == 0 => {}
                _ => {
                    clear_part(part);
                    last_error = Some(DownloadError::RangeMismatch);
                    stalled += 1;
                    continue;
                }
            }
        }
        if effective_resume > 0 {
            // Identity must be provable, or the bytes on disk may belong to
            // another revision of the resource.
            let provable = match (identity.as_ref(), response.etag.as_deref(), response.last_modified.as_deref()) {
                (Some(stored), Some(now), _) => stored.etag.as_deref() == Some(now),
                (Some(stored), None, Some(now)) => stored.last_modified.as_deref() == Some(now),
                // Neither header was sent the first time or now: restart.
                _ => false,
            };
            if !provable {
                // The bytes on disk belong to a resource that can no longer be
                // proven identical, so they are dropped and the next attempt
                // starts from zero.
                clear_part(part);
                last_error = Some(DownloadError::IdentityChanged);
                continue;
            }
        }

        // `Content-Length`, when the server sends it, must agree with what the
        // pinned size says this response should contain.
        let expected_body = expectation
            .expected_size
            .saturating_sub(effective_resume);
        if let Some(length) = response.content_length {
            if length != expected_body {
                return Err(DownloadError::ContentLengthMismatch);
            }
        }

        let start_bytes = effective_resume;
        // A failure inside the writer (a local I/O error) is not retryable; a
        // failure of the *stream* comes back as `Interrupted`.
        match write_body(
            part,
            response,
            effective_resume,
            expectation,
            &mut progress,
            &mut cancelled,
        )? {
            BodyOutcome::Complete { bytes } => {
                if bytes != expectation.expected_size {
                    // The stream ended early. The partial file is kept so the
                    // next attempt can resume it.
                    last_error = Some(DownloadError::SizeMismatch);
                    if bytes <= start_bytes {
                        stalled += 1;
                    } else {
                        stalled = 0;
                    }
                    continue;
                }
                let actual = sha256_file(part)?;
                if !actual.eq_ignore_ascii_case(&expectation.sha256) {
                    // The bytes are provably wrong; keeping them would only
                    // make the next attempt resume from bad data.
                    clear_part(part);
                    return Err(DownloadError::HashMismatch);
                }
                // Verified: the `.part` file is left for the caller to promote,
                // and only the resume bookkeeping is dropped.
                clear_identity(part);
                return Ok(DownloadOutcome {
                    bytes,
                    resumed_from: start_bytes,
                    fetches: attempts,
                });
            }
            BodyOutcome::Interrupted { bytes, error } => {
                if bytes <= start_bytes {
                    stalled += 1;
                } else {
                    stalled = 0;
                }
                last_error = Some(error);
                continue;
            }
        }
    }
}

enum BodyOutcome {
    Complete { bytes: u64 },
    Interrupted { bytes: u64, error: DownloadError },
}

/// Streams a response body into the part file.
///
/// The byte cap is enforced here rather than trusted from the headers, so a
/// missing or dishonest `Content-Length` cannot fill the disk.
fn write_body<F, C>(
    part: &Path,
    mut response: TransportResponse,
    resume_from: u64,
    expectation: &ArtifactExpectation,
    progress: &mut F,
    cancelled: &mut C,
) -> Result<BodyOutcome, DownloadError>
where
    F: FnMut(DownloadProgress),
    C: FnMut() -> bool,
{
    let mut options = OpenOptions::new();
    options.create(true).write(true);
    if resume_from == 0 {
        options.truncate(true);
    }
    let mut file = options.open(part).map_err(|_| DownloadError::Io)?;
    if resume_from > 0 {
        file.set_len(resume_from).map_err(|_| DownloadError::Io)?;
        file.seek(SeekFrom::Start(resume_from))
            .map_err(|_| DownloadError::Io)?;
    }
    if resume_from == 0 {
        // The identity is stored before any byte is written, so a crash later
        // still leaves enough information to resume.
        write_identity(
            part,
            &PartIdentity {
                etag: response.etag.clone(),
                last_modified: response.last_modified.clone(),
                expected_size: expectation.expected_size,
                sha256: expectation.sha256.to_ascii_lowercase(),
            },
        )?;
    }

    let mut written = resume_from;
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    loop {
        if cancelled() {
            let _ = file.sync_all();
            return Err(DownloadError::Cancelled);
        }
        let count = match response.body.read(&mut buffer) {
            Ok(count) => count,
            Err(error) => {
                let _ = file.sync_all();
                return Ok(BodyOutcome::Interrupted {
                    bytes: written,
                    error: if error.kind() == std::io::ErrorKind::TimedOut {
                        DownloadError::Timeout
                    } else {
                        DownloadError::Network
                    },
                });
            }
        };
        if count == 0 {
            break;
        }
        written = match written.checked_add(count as u64) {
            Some(total) => total,
            None => return Err(DownloadError::TooLarge),
        };
        // Independent of every header the server sent.
        if written > expectation.expected_size {
            return Err(DownloadError::TooLarge);
        }
        file.write_all(&buffer[..count])
            .map_err(|_| DownloadError::Io)?;
        progress(DownloadProgress {
            downloaded: written,
            total: expectation.expected_size,
        });
    }
    file.sync_all().map_err(|_| DownloadError::Io)?;
    Ok(BodyOutcome::Complete { bytes: written })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::local::setup::fake_http::{FakeResponse, FakeServer};
    use tempfile::tempdir;

    /// Deterministic body of `len` bytes, distinct for each seed.
    fn body(len: usize, seed: u8) -> Vec<u8> {
        crate::ai::local::setup::fixtures::bytes(len, seed)
    }

    fn expectation_for(server: &FakeServer, bytes: &[u8], name: &str) -> ArtifactExpectation {
        ArtifactExpectation {
            url: server.url(name),
            filename: name.to_string(),
            expected_size: bytes.len() as u64,
            sha256: sha256_of(bytes),
        }
    }

    fn sha256_of(bytes: &[u8]) -> String {
        let mut digest = Sha256::new();
        digest.update(bytes);
        format!("{:x}", digest.finalize())
    }

    fn part_path(directory: &Path, name: &str) -> PathBuf {
        directory.join(format!("{name}.part"))
    }

    fn run(
        _server: &FakeServer,
        expectation: &ArtifactExpectation,
        part: &Path,
    ) -> Result<DownloadOutcome, DownloadError> {
        let transport = ReqwestTransport::new().unwrap();
        download_artifact(
            &transport,
            DownloadRequest {
                expectation,
                part,
                trust: ArtifactTrust::LoopbackTesting,
            },
            |_| {},
            || false,
        )
    }

    #[test]
    fn a_complete_download_is_verified_and_left_as_a_part_file() {
        let server = FakeServer::start();
        let payload = body(256 * 1024, 7);
        server.push(FakeResponse::full(payload.clone()));
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");

        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(outcome.bytes, payload.len() as u64);
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(outcome.fetches, 1);
        assert_eq!(fs::read(&part).unwrap(), payload);
        // Nothing is activated here, and the identity file does not survive.
        assert!(!identity_path(&part).exists());
    }

    #[test]
    fn progress_is_reported_in_bytes_and_ends_at_the_total() {
        let server = FakeServer::start();
        let payload = body(200 * 1024, 3);
        server.push(FakeResponse::full(payload.clone()));
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");

        let mut seen = Vec::new();
        let transport = ReqwestTransport::new().unwrap();
        download_artifact(
            &transport,
            DownloadRequest {
                expectation: &expectation,
                part: &part,
                trust: ArtifactTrust::LoopbackTesting,
            },
            |progress| seen.push(progress),
            || false,
        )
        .unwrap();

        assert!(!seen.is_empty());
        assert!(seen.windows(2).all(|pair| pair[0].downloaded <= pair[1].downloaded));
        assert_eq!(seen.last().unwrap().downloaded, payload.len() as u64);
        assert!(seen.iter().all(|progress| progress.total == payload.len() as u64));
    }

    #[test]
    fn a_pinned_artifact_is_the_only_thing_the_application_may_fetch() {
        let model = managed_model_manifest().artifact;
        let pinned = ArtifactExpectation::from(model);
        assert!(validate_expectation(&pinned, ArtifactTrust::Pinned).is_ok());
        assert!(is_pinned(&pinned));

        // A well-formed HTTPS URL that is not a compiled-in manifest.
        let other = ArtifactExpectation {
            url: "https://example.test/model.gguf".to_string(),
            filename: "model.gguf".to_string(),
            expected_size: 1,
            sha256: "a".repeat(64),
        };
        assert_eq!(
            validate_expectation(&other, ArtifactTrust::Pinned),
            Err(DownloadError::RefusedUrl)
        );
        // Plain HTTP is refused outright.
        let insecure = ArtifactExpectation {
            url: "http://example.test/model.gguf".to_string(),
            ..other.clone()
        };
        assert_eq!(
            validate_expectation(&insecure, ArtifactTrust::Pinned),
            Err(DownloadError::RefusedUrl)
        );
        // The loopback transport refuses a non-loopback host, so a test-only
        // convenience cannot reach the internet.
        assert_eq!(
            validate_expectation(&other, ArtifactTrust::LoopbackTesting),
            Err(DownloadError::RefusedUrl)
        );
        // A malformed description never reaches a socket.
        for broken in [
            ArtifactExpectation {
                expected_size: 0,
                ..pinned.clone()
            },
            ArtifactExpectation {
                sha256: "xyz".to_string(),
                ..pinned.clone()
            },
            ArtifactExpectation {
                filename: "../escape.gguf".to_string(),
                ..pinned.clone()
            },
        ] {
            assert_eq!(
                validate_expectation(&broken, ArtifactTrust::Pinned),
                Err(DownloadError::RefusedUrl)
            );
        }
    }

    #[test]
    fn a_non_https_url_never_opens_a_socket() {
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = ArtifactExpectation {
            url: "http://127.0.0.1:1/model.gguf".to_string(),
            filename: "model.gguf".to_string(),
            expected_size: 4,
            sha256: "b".repeat(64),
        };
        let transport = ReqwestTransport::new().unwrap();
        let result = download_artifact(
            &transport,
            DownloadRequest {
                expectation: &expectation,
                part: &part,
                trust: ArtifactTrust::Pinned,
            },
            |_| {},
            || false,
        );
        assert_eq!(result, Err(DownloadError::RefusedUrl));
        assert!(!part.exists());
    }

    #[test]
    fn the_destination_name_must_be_the_artifact_name() {
        let server = FakeServer::start();
        let payload = body(1024, 1);
        let expectation = expectation_for(&server, &payload, "model.gguf");
        let directory = tempdir().unwrap();
        let wrong = directory.path().join("other.gguf.part");
        assert_eq!(run(&server, &expectation, &wrong), Err(DownloadError::Destination));
    }

    #[test]
    fn a_wrong_content_length_is_refused_before_the_body_is_read() {
        let server = FakeServer::start();
        let payload = body(4096, 2);
        // The header claims one byte fewer than the pinned size.
        server.push(FakeResponse::full_with_length(
            payload.clone(),
            payload.len() as u64 - 1,
        ));
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");
        assert_eq!(
            run(&server, &expectation, &part),
            Err(DownloadError::ContentLengthMismatch)
        );
        // A header that disagrees with the manifest is not trusted enough to
        // leave bytes behind.
        assert!(!part.exists());
    }

    #[test]
    fn a_missing_content_length_still_downloads_and_is_bounded_by_the_pinned_size() {
        let server = FakeServer::start();
        let payload = body(4096, 4);
        server.push(FakeResponse::close_delimited(payload.clone()));
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");
        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(outcome.bytes, 4096);
    }

    #[test]
    fn a_body_longer_than_the_pinned_size_is_stopped_by_the_hard_cap() {
        let server = FakeServer::start();
        let payload = body(4096, 5);
        // No Content-Length at all, and more bytes than the manifest allows.
        server.push(FakeResponse::close_delimited(
            body(8192, 6),
        ));
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");
        assert_eq!(run(&server, &expectation, &part), Err(DownloadError::TooLarge));
    }

    #[test]
    fn a_short_body_is_a_size_mismatch_and_never_becomes_a_complete_file() {
        let server = FakeServer::start();
        let payload = body(4096, 8);
        let short = &payload[..2048];
        // No `Content-Length`, so the length is only discovered from the end of
        // the body: the download must notice the short file itself.
        for _ in 0..3 {
            server.push(FakeResponse::close_delimited(short.to_vec()));
        }
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");
        assert_eq!(run(&server, &expectation, &part), Err(DownloadError::SizeMismatch));
        // The bytes that did arrive are kept, because a resume can use them.
        assert!(part.exists());
    }

    #[test]
    fn a_hash_mismatch_removes_the_file_it_cannot_trust() {
        let server = FakeServer::start();
        let payload = body(4096, 9);
        server.push(FakeResponse::full(payload.clone()));
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let mut expectation = expectation_for(&server, &payload, "model.gguf");
        expectation.sha256 = "c".repeat(64);
        assert_eq!(run(&server, &expectation, &part), Err(DownloadError::HashMismatch));
        assert!(!part.exists());
    }

    #[test]
    fn a_broken_connection_is_retried_and_the_transfer_resumes() {
        let server = FakeServer::start();
        let payload = body(300 * 1024, 10);
        // The first response advertises the full body but sends only a quarter.
        server.push(FakeResponse::truncated(payload.clone(), payload.len() / 4));
        // The second answers a ranged request for the rest.
        server.push(FakeResponse::ranged(payload.clone()));
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");

        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(outcome.bytes, payload.len() as u64);
        assert_eq!(outcome.fetches, 2);
        assert_eq!(outcome.resumed_from, (payload.len() / 4) as u64);
        assert_eq!(fs::read(&part).unwrap(), payload);
        // The second request really did carry a Range header.
        assert!(server.requests()[1].range_start == Some((payload.len() / 4) as u64));
    }

    #[test]
    fn a_server_that_answers_two_hundred_to_a_ranged_request_is_never_appended_to() {
        let server = FakeServer::start();
        let payload = body(200 * 1024, 11);
        // A partial file and its identity already exist.
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let prefix = &payload[..payload.len() / 2];
        fs::write(&part, prefix).unwrap();
        write_identity(
            &part,
            &PartIdentity {
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                expected_size: payload.len() as u64,
                sha256: sha256_of(&payload),
            },
        )
        .unwrap();
        // The server ignores the Range header and sends the whole body again.
        server.push(FakeResponse::full(payload.clone()));
        let expectation = expectation_for(&server, &payload, "model.gguf");

        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(outcome.bytes, payload.len() as u64);
        // The result is the resource, not prefix + resource.
        assert_eq!(fs::read(&part).unwrap(), payload);
        assert_eq!(outcome.resumed_from, 0);
    }

    #[test]
    fn a_changed_etag_forces_a_restart_instead_of_a_resume() {
        let server = FakeServer::start();
        let payload = body(200 * 1024, 12);
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let prefix = &payload[..payload.len() / 2];
        fs::write(&part, prefix).unwrap();
        write_identity(
            &part,
            &PartIdentity {
                etag: Some("\"old\"".to_string()),
                last_modified: None,
                expected_size: payload.len() as u64,
                sha256: sha256_of(&payload),
            },
        )
        .unwrap();
        // The range is honoured, but the resource identity changed.
        server.push(FakeResponse::ranged_with_etag(payload.clone(), "\"new\""));
        // The restart gets a plain, full response.
        server.push(FakeResponse::full(payload.clone()));
        let expectation = expectation_for(&server, &payload, "model.gguf");

        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(fs::read(&part).unwrap(), payload);
        assert_eq!(server.requests().len(), 2);
        // The second request did not ask for a range: the identity could not be
        // proven, so the download started over.
        assert_eq!(server.requests()[1].range_start, None);
    }

    #[test]
    fn a_range_that_does_not_match_the_partial_file_is_refused() {
        let server = FakeServer::start();
        let payload = body(200 * 1024, 13);
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        fs::write(&part, &payload[..1024]).unwrap();
        write_identity(
            &part,
            &PartIdentity {
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                expected_size: payload.len() as u64,
                sha256: sha256_of(&payload),
            },
        )
        .unwrap();
        // The server answers with a range that starts somewhere else entirely.
        server.push(FakeResponse::ranged_from(payload.clone(), 4096, "\"v1\""));
        server.push(FakeResponse::full(payload.clone()));
        let expectation = expectation_for(&server, &payload, "model.gguf");
        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(fs::read(&part).unwrap(), payload);
        assert_eq!(outcome.resumed_from, 0);
    }

    #[test]
    fn a_partial_file_without_a_stored_identity_is_not_appended_to() {
        let server = FakeServer::start();
        let payload = body(100 * 1024, 14);
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        fs::write(&part, &payload[..1024]).unwrap();
        // No identity file: the previous run left nothing to prove identity with.
        server.push(FakeResponse::full(payload.clone()));
        let expectation = expectation_for(&server, &payload, "model.gguf");
        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(outcome.resumed_from, 0);
        assert_eq!(fs::read(&part).unwrap(), payload);
        assert_eq!(server.requests()[0].range_start, None);
    }

    #[test]
    fn a_complete_part_file_is_settled_by_hashing_it_instead_of_downloading_again() {
        let server = FakeServer::start();
        let payload = body(64 * 1024, 15);
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        fs::write(&part, &payload).unwrap();
        let expectation = expectation_for(&server, &payload, "model.gguf");
        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(outcome.fetches, 0);
        assert_eq!(outcome.bytes, payload.len() as u64);
        assert!(server.requests().is_empty());
    }

    #[test]
    fn a_part_file_longer_than_the_resource_is_discarded_and_refetched() {
        let server = FakeServer::start();
        let payload = body(32 * 1024, 16);
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        fs::write(&part, body(64 * 1024, 17)).unwrap();
        server.push(FakeResponse::full(payload.clone()));
        let expectation = expectation_for(&server, &payload, "model.gguf");
        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(outcome.fetches, 1);
        assert_eq!(fs::read(&part).unwrap(), payload);
    }

    #[test]
    fn a_range_not_satisfiable_answer_restarts_the_download() {
        let server = FakeServer::start();
        let payload = body(48 * 1024, 18);
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        fs::write(&part, &payload[..1024]).unwrap();
        write_identity(
            &part,
            &PartIdentity {
                etag: Some("\"v1\"".to_string()),
                last_modified: None,
                expected_size: payload.len() as u64,
                sha256: sha256_of(&payload),
            },
        )
        .unwrap();
        server.push(FakeResponse::status(416));
        server.push(FakeResponse::ignore_range(payload.clone()));
        let expectation = expectation_for(&server, &payload, "model.gguf");
        let outcome = run(&server, &expectation, &part).unwrap();
        assert_eq!(fs::read(&part).unwrap(), payload);
        assert_eq!(outcome.resumed_from, 0);
    }

    #[test]
    fn cancellation_stops_the_download_and_keeps_the_partial_file() {
        let server = FakeServer::start();
        let payload = body(4 * 1024 * 1024, 19);
        server.push(FakeResponse::throttled(payload.clone(), 32 * 1024));
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");

        let transport = ReqwestTransport::new().unwrap();
        let result = download_artifact(
            &transport,
            DownloadRequest {
                expectation: &expectation,
                part: &part,
                trust: ArtifactTrust::LoopbackTesting,
            },
            |_| {},
            || part.exists() && fs::metadata(&part).map(|m| m.len()).unwrap_or(0) > 0,
        );
        assert_eq!(result, Err(DownloadError::Cancelled));
        // The partial file survives, which is what makes the retry a resume.
        assert!(part.exists());
        let kept = fs::metadata(&part).unwrap().len();
        assert!(kept > 0);
        assert!(kept < payload.len() as u64);
    }

    #[test]
    fn retries_are_bounded_and_a_stalled_endpoint_gives_up() {
        let server = FakeServer::start();
        let payload = body(64 * 1024, 20);
        // Five failing answers, but only three requests may be made.
        for _ in 0..5 {
            server.push(FakeResponse::status(500));
        }
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");
        let error = run(&server, &expectation, &part).unwrap_err();
        assert_eq!(error, DownloadError::Network);
        assert_eq!(server.requests().len(), MAX_DOWNLOAD_ATTEMPTS as usize);
    }

    #[test]
    fn an_endpoint_that_never_progresses_is_reported_as_no_progress() {
        let server = FakeServer::start();
        let payload = body(256 * 1024, 21);
        // 200 with a body that ends immediately: no bytes, every time.
        for _ in 0..5 {
            server.push(FakeResponse::close_delimited(Vec::new()));
        }
        let directory = tempdir().unwrap();
        let part = part_path(directory.path(), "model.gguf");
        let expectation = expectation_for(&server, &payload, "model.gguf");
        let error = run(&server, &expectation, &part).unwrap_err();
        assert_eq!(error, DownloadError::NoProgress);
        assert!(server.requests().len() <= MAX_STALLED_ATTEMPTS as usize + 1);
    }

    #[test]
    fn content_range_parsing_covers_the_shapes_a_server_sends() {
        assert_eq!(
            parse_content_range("bytes 100-499/1234"),
            Some(ContentRange {
                start: 100,
                end: 499,
                total: Some(1234)
            })
        );
        assert_eq!(
            parse_content_range("bytes 0-0/*"),
            Some(ContentRange {
                start: 0,
                end: 0,
                total: None
            })
        );
        assert_eq!(parse_content_range("items 1-2/3"), None);
        assert_eq!(parse_content_range("bytes 9-2/3"), None);
        assert_eq!(parse_content_range("bytes a-b/c"), None);
    }

    #[test]
    fn error_codes_are_stable_and_retryability_is_explicit() {
        assert_eq!(DownloadError::HashMismatch.code(), "hash_mismatch");
        assert_eq!(DownloadError::Cancelled.code(), "cancelled");
        assert!(DownloadError::Network.is_retryable());
        assert!(DownloadError::Timeout.is_retryable());
        assert!(!DownloadError::HashMismatch.is_retryable());
        assert!(!DownloadError::Cancelled.is_retryable());
        assert!(!DownloadError::TooLarge.is_retryable());
    }

    #[test]
    fn hashing_a_file_matches_the_expected_digest() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("x.bin");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
