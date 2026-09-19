//! The portable backup container: one versioned file, documented and bounded.
//!
//! # Why not an archive format
//!
//! A zip or a tar reader is a large parser that also has to be trusted with
//! untrusted input. This container is deliberately small enough to read in one
//! sitting:
//!
//! ```text
//! magic          8 bytes   "JARVISBK"
//! format         u16 LE    1
//! header_len     u32 LE    length of the JSON header that follows
//! header         JSON      cleartext, see `ContainerHeader`
//! entries        repeated, in manifest order:
//!   record_len   u32 LE    length of the record that follows
//!   record       version byte, 24-byte nonce, AEAD ciphertext
//! ```
//!
//! # What is authenticated, and what is not
//!
//! * the **manifest** (logical names, sizes, SHA-256 of every entry, schema
//!   versions) is authenticated by [`ContainerHeader::manifest_sha256`], and
//!   that hash is part of the associated data of every encrypted chunk;
//! * the **content** of every entry is authenticated twice: by the AEAD tag of
//!   each chunk, and by the SHA-256 of the whole plaintext entry in the manifest;
//! * the chunk index, the logical name, the plaintext length, and whether the
//!   chunk is the last one are all bound into the associated data, so entries
//!   cannot be reordered, renamed, truncated, or swapped;
//! * the **header in the clear** carries no secret: a format name, a timestamp,
//!   an application version, the password-protected key envelope (which is
//!   useless without the password), and the manifest — and the manifest names
//!   logical components, never a user path and never a note.
//!
//! # Bounds
//!
//! A container that declares more entries than [`Limits::max_entries`], an entry
//! larger than [`Limits::max_entry_bytes`], a total larger than
//! [`Limits::max_total_bytes`], or a header larger than
//! [`Limits::max_header_bytes`] is refused before anything is allocated or
//! written. Every chunk length is checked against the size the manifest states,
//! so a container cannot ask this reader for an unbounded amount of memory.

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::sync::crypto::{
    decrypt, derive_purpose_key, encrypt, import_backup, KeyPurpose, MasterKey, PortableKeyBackup,
    NONCE_BYTES,
};

use super::error::BackupError;

/// First eight bytes of every container.
pub const MAGIC: [u8; 8] = *b"JARVISBK";
/// Container format version this build writes.
pub const FORMAT_VERSION: u16 = 1;
/// Value of [`ContainerHeader::format`].
pub const FORMAT_NAME: &str = "jarvis-backup";
/// Plaintext bytes per encrypted chunk.
pub const CHUNK_BYTES: usize = 1024 * 1024;
/// Bytes of the fixed prefix before the header.
pub const PREFIX_BYTES: u64 = 8 + 2 + 4;
/// Longest logical entry name.
pub const MAX_NAME_BYTES: usize = 96;

/// What a container is allowed to declare.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_entries: usize,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    pub max_header_bytes: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 64,
            max_entry_bytes: 256 * 1024 * 1024,
            max_total_bytes: 512 * 1024 * 1024,
            max_header_bytes: 64 * 1024,
        }
    }
}

/// The bounds every read uses unless a caller says otherwise.
pub const LIMITS: Limits = Limits {
    max_entries: 64,
    max_entry_bytes: 256 * 1024 * 1024,
    max_total_bytes: 512 * 1024 * 1024,
    max_header_bytes: 64 * 1024,
};

/// What kind of data an entry holds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ComponentKind {
    /// A SQLite database, snapshotted consistently.
    Sqlite,
    /// A small settings document, read atomically as it was written.
    Document,
    /// The portable, password-protected master-key envelope.
    PortableKey,
}

/// One component this build can put in a container.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManifestEntry {
    /// Logical name, `component/file`: never a user path.
    pub name: String,
    pub kind: ComponentKind,
    /// Size of the plaintext, in bytes.
    pub bytes: u64,
    /// SHA-256 of the plaintext, lowercase hex.
    pub sha256: String,
    /// SQLite `user_version`/schema the snapshot declares, when it is a database.
    pub schema_version: Option<i64>,
}

/// The list of what a container holds.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackupManifest {
    pub entries: Vec<ManifestEntry>,
    pub total_bytes: u64,
}

impl BackupManifest {
    /// The entry with this logical name.
    pub fn entry(&self, name: &str) -> Option<&ManifestEntry> {
        self.entries.iter().find(|entry| entry.name == name)
    }

    /// Whether every name is unique and safe.
    fn validate_names(&self, limits: &Limits) -> Result<(), BackupError> {
        if self.entries.len() > limits.max_entries {
            return Err(BackupError::TooLarge);
        }
        let mut seen: HashSet<&str> = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            if !is_safe_entry_name(&entry.name) {
                return Err(BackupError::UnsafeEntryName);
            }
            if !seen.insert(entry.name.as_str()) {
                return Err(BackupError::DuplicateEntryName);
            }
            if entry.bytes > limits.max_entry_bytes {
                return Err(BackupError::TooLarge);
            }
            if entry.sha256.len() != 64 || !entry.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(BackupError::ManifestMismatch);
            }
        }
        if self.total_bytes > limits.max_total_bytes {
            return Err(BackupError::TooLarge);
        }
        Ok(())
    }
}

/// The cleartext part of a container.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContainerHeader {
    pub format: String,
    pub format_version: u16,
    /// RFC 3339, UTC.
    pub created_at: String,
    pub app_version: String,
    pub cipher: String,
    pub kdf: String,
    /// Hex of the portable key envelope JSON. Useless without the password.
    pub envelope: String,
    pub manifest: BackupManifest,
    /// SHA-256 of the canonical manifest JSON, lowercase hex.
    pub manifest_sha256: String,
}

impl ContainerHeader {
    /// What the window shows before anything is restored: no secrets.
    pub fn preview(&self) -> BackupPreview {
        let mut warnings = Vec::new();
        if self.manifest.entry("vault/vault.sqlite3").is_none() {
            warnings.push("no_vault".to_string());
        }
        if self.manifest.entry("notes/sync.sqlite3").is_none() {
            warnings.push("no_notes".to_string());
        }
        if self.manifest.entry("key/portable-envelope.json").is_none() {
            warnings.push("no_key_envelope".to_string());
        }
        BackupPreview {
            format_version: self.format_version,
            created_at: self.created_at.clone(),
            app_version: self.app_version.clone(),
            total_bytes: self.manifest.total_bytes,
            entries: self
                .manifest
                .entries
                .iter()
                .map(|entry| PreviewEntry {
                    name: entry.name.clone(),
                    kind: entry.kind,
                    bytes: entry.bytes,
                })
                .collect(),
            warnings,
        }
    }
}

/// One line of the preview: a logical name and a size, and nothing else.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreviewEntry {
    pub name: String,
    pub kind: ComponentKind,
    pub bytes: u64,
}

/// What the window is told about a container before a restore.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackupPreview {
    pub format_version: u16,
    pub created_at: String,
    pub app_version: String,
    pub total_bytes: u64,
    pub entries: Vec<PreviewEntry>,
    /// Content-free codes: `no_vault`, `no_notes`, `no_key_envelope`.
    pub warnings: Vec<String>,
}

/// A component and where its snapshot lives.
#[derive(Clone, Debug)]
pub struct EntrySource {
    pub name: String,
    pub kind: ComponentKind,
    pub path: PathBuf,
    pub schema_version: Option<i64>,
}

/// What a written container holds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerSummary {
    pub format_version: u16,
    pub created_at: String,
    pub app_version: String,
    pub entries: Vec<ManifestEntry>,
    pub total_bytes: u64,
}

impl ContainerSummary {
    pub fn preview(&self) -> BackupPreview {
        BackupPreview {
            format_version: self.format_version,
            created_at: self.created_at.clone(),
            app_version: self.app_version.clone(),
            total_bytes: self.total_bytes,
            entries: self
                .entries
                .iter()
                .map(|entry| PreviewEntry {
                    name: entry.name.clone(),
                    kind: entry.kind,
                    bytes: entry.bytes,
                })
                .collect(),
            warnings: Vec::new(),
        }
    }
}

/// Whether a logical name may be stored and later written to a file.
///
/// This is the path-traversal gate. A name is a logical `component/file` pair:
/// lowercase ASCII, digits, dot, dash, underscore and one separator; no leading
/// or trailing separator; no empty, `.` or `..` component; no backslash, colon,
/// or NUL — so an NTFS alternate data stream cannot be named either.
pub fn is_safe_entry_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return false;
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return false;
    }
    let allowed =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-');
    if !name.split('/').all(|part| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part.len() <= 48
            && part.chars().all(allowed)
    }) {
        return false;
    }
    // Reserved Windows device names, in any directory, with or without extension.
    const RESERVED: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    !name.split('/').any(|part| {
        let stem = part.split('.').next().unwrap_or(part);
        RESERVED.contains(&stem)
    })
}

/// The associated data of one chunk: everything that must not change.
fn chunk_context(manifest_sha256: &str, name: &str, index: u64, last: bool, len: u32) -> Vec<u8> {
    format!(
        "jarvis-backup-chunk|{manifest_sha256}|{name}|{index}|{}|{len}",
        u8::from(last)
    )
    .into_bytes()
}

/// Lowercase hex, without a dependency.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

/// Parses lowercase or uppercase hex; anything else is a refusal.
pub fn from_hex(text: &str) -> Result<Vec<u8>, BackupError> {
    if !text.len().is_multiple_of(2) {
        return Err(BackupError::ManifestMismatch);
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks(2) {
        let high = (pair[0] as char).to_digit(16);
        let low = (pair[1] as char).to_digit(16);
        match (high, low) {
            (Some(high), Some(low)) => out.push(((high << 4) | low) as u8),
            _ => return Err(BackupError::ManifestMismatch),
        }
    }
    Ok(out)
}

/// SHA-256 of a byte slice, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    to_hex(&hasher.finalize())
}

/// SHA-256 of a file, read in bounded chunks.
fn sha256_of_file(path: &Path) -> Result<(u64, String), BackupError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        hasher.update(&buffer[..read]);
    }
    Ok((total, to_hex(&hasher.finalize())))
}

/// The canonical bytes of a manifest, which is what its hash covers.
pub fn canonical_manifest(manifest: &BackupManifest) -> Result<Vec<u8>, BackupError> {
    Ok(serde_json::to_vec(manifest)?)
}

/// Writes a container.
///
/// The caller has already put every snapshot in place and has already chosen a
/// temporary destination: this function writes the whole file, flushes it, and
/// leaves the rename to the caller, so a failed export never leaves a
/// half-written container under the name the user chose.
pub fn write_container(
    destination: &Path,
    entries: &[EntrySource],
    envelope: &PortableKeyBackup,
    key: &MasterKey,
    app_version: &str,
    created_at: &str,
    limits: &Limits,
) -> Result<ContainerSummary, BackupError> {
    if entries.len() > limits.max_entries {
        return Err(BackupError::TooLarge);
    }
    if envelope.format_version == 0 {
        return Err(BackupError::WrongPasswordOrDamaged);
    }

    // Pass one: measure and hash every snapshot, and check the manifest rules
    // before a single byte is written.
    let mut manifest = BackupManifest::default();
    for entry in entries {
        let (bytes, sha256) = sha256_of_file(&entry.path)?;
        if bytes > limits.max_entry_bytes {
            return Err(BackupError::TooLarge);
        }
        manifest.total_bytes = manifest.total_bytes.saturating_add(bytes);
        manifest.entries.push(ManifestEntry {
            name: entry.name.clone(),
            kind: entry.kind,
            bytes,
            sha256,
            schema_version: entry.schema_version,
        });
    }
    manifest.validate_names(limits)?;
    if manifest.total_bytes > limits.max_total_bytes {
        return Err(BackupError::TooLarge);
    }

    let manifest_bytes = canonical_manifest(&manifest)?;
    let manifest_sha256 = sha256_hex(&manifest_bytes);
    let envelope_json = serde_json::to_vec(envelope)?;
    let header = ContainerHeader {
        format: FORMAT_NAME.to_string(),
        format_version: FORMAT_VERSION,
        created_at: created_at.to_string(),
        app_version: app_version.to_string(),
        cipher: "xchacha20poly1305".to_string(),
        kdf: KeyPurpose::Backup.label().to_string(),
        envelope: to_hex(&envelope_json),
        manifest,
        manifest_sha256: manifest_sha256.clone(),
    };
    let header_bytes = serde_json::to_vec(&header)?;
    if header_bytes.len() as u64 > u64::from(limits.max_header_bytes) {
        return Err(BackupError::TooLarge);
    }

    // Pass two: the container itself, one bounded chunk at a time.
    let mut file = std::fs::File::create(destination)?;
    file.write_all(&MAGIC)?;
    file.write_all(&FORMAT_VERSION.to_le_bytes())?;
    file.write_all(&(header_bytes.len() as u32).to_le_bytes())?;
    file.write_all(&header_bytes)?;

    for entry in entries {
        let mut source = std::fs::File::open(&entry.path)?;
        let mut buffer = vec![0u8; CHUNK_BYTES];
        let mut index = 0u64;
        let mut written = 0u64;
        let total = header
            .manifest
            .entry(&entry.name)
            .map(|entry| entry.bytes)
            .unwrap_or(0);
        loop {
            let wanted = (total - written).min(CHUNK_BYTES as u64) as usize;
            let read = read_exact_up_to(&mut source, &mut buffer[..wanted])?;
            if read == 0 && written > 0 {
                // The file shrank between the two passes: refuse rather than
                // write a container that contradicts its own manifest.
                return Err(BackupError::ContentMismatch);
            }
            let last = written + read as u64 >= total;
            let context = chunk_context(&manifest_sha256, &entry.name, index, last, read as u32);
            let record = encrypt(key, &context, &buffer[..read])?;
            let encoded = record.encode();
            file.write_all(&(encoded.len() as u32).to_le_bytes())?;
            file.write_all(&encoded)?;
            written += read as u64;
            index += 1;
            if last || read < wanted {
                break;
            }
        }
        if written != total {
            return Err(BackupError::ContentMismatch);
        }
    }

    file.flush()?;
    // Durability is the caller's contract too, but a container that is not on
    // disk is not a backup: sync before the rename.
    file.sync_all()?;
    drop(file);

    Ok(ContainerSummary {
        format_version: FORMAT_VERSION,
        created_at: header.created_at,
        app_version: header.app_version,
        entries: header.manifest.entries,
        total_bytes: header.manifest.total_bytes,
    })
}

/// Reads exactly `buffer.len()` bytes unless the file ends first.
fn read_exact_up_to(file: &mut std::fs::File, mut buffer: &mut [u8]) -> Result<usize, BackupError> {
    let mut total = 0;
    while !buffer.is_empty() {
        let read = file.read(buffer)?;
        if read == 0 {
            break;
        }
        total += read;
        buffer = &mut buffer[read..];
    }
    Ok(total)
}

/// Reads and checks the cleartext header, without a password.
///
/// Everything that can be checked without the key is checked here: the magic,
/// the version, the header bounds, the manifest rules, and the manifest hash.
pub fn read_header(
    path: &Path,
    limits: &Limits,
) -> Result<(ContainerHeader, [u8; 32]), BackupError> {
    let mut file = std::fs::File::open(path)?;
    let file_bytes = file.metadata()?.len();
    if file_bytes < PREFIX_BYTES {
        return Err(BackupError::NotAContainer);
    }
    let mut magic = [0u8; 8];
    file.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(BackupError::NotAContainer);
    }
    let mut version = [0u8; 2];
    file.read_exact(&mut version)?;
    let version = u16::from_le_bytes(version);
    if version != FORMAT_VERSION {
        return Err(BackupError::UnsupportedVersion);
    }
    let mut length = [0u8; 4];
    file.read_exact(&mut length)?;
    let header_len = u32::from_le_bytes(length);
    if header_len == 0 || header_len > limits.max_header_bytes {
        return Err(BackupError::TooLarge);
    }
    let mut header_bytes = vec![0u8; header_len as usize];
    file.read_exact(&mut header_bytes)?;
    let header: ContainerHeader = serde_json::from_slice(&header_bytes)?;
    if header.format != FORMAT_NAME || header.format_version != FORMAT_VERSION {
        return Err(BackupError::UnsupportedVersion);
    }
    if header.cipher != "xchacha20poly1305" || header.kdf != KeyPurpose::Backup.label() {
        return Err(BackupError::UnsupportedVersion);
    }
    header.manifest.validate_names(limits)?;
    let manifest_bytes = canonical_manifest(&header.manifest)?;
    let computed = sha256_hex(&manifest_bytes);
    if computed != header.manifest_sha256 {
        return Err(BackupError::ManifestMismatch);
    }
    let mut sha = [0u8; 32];
    let decoded = from_hex(&header.manifest_sha256)?;
    if decoded.len() != 32 {
        return Err(BackupError::ManifestMismatch);
    }
    sha.copy_from_slice(&decoded);
    Ok((header, sha))
}

/// Unlocks the container: the password must open the key envelope.
pub fn unlock(header: &ContainerHeader, password: &[u8]) -> Result<MasterKey, BackupError> {
    let envelope_bytes = from_hex(&header.envelope)?;
    let envelope: PortableKeyBackup = serde_json::from_slice(&envelope_bytes)?;
    let master = import_backup(&envelope, password)?;
    Ok(derive_purpose_key(&master, KeyPurpose::Backup)?)
}

/// Where the verification writes what it decrypts.
enum Sink<'a> {
    /// Nothing: this is a dry run, and only the checksums are computed.
    Verify,
    /// One file per entry, under this directory.
    Directory(&'a Path),
}

/// Decrypts every entry, checks every checksum, and optionally writes the files.
///
/// This is the one function that reads a container's payload, so the dry run the
/// window shows and the extraction that precedes a restore cannot disagree.
fn walk(
    path: &Path,
    password: &[u8],
    sink: Sink<'_>,
    cancel: &AtomicBool,
    limits: &Limits,
) -> Result<ContainerSummary, BackupError> {
    let (header, _manifest_sha) = read_header(path, limits)?;
    let key = unlock(&header, password)?;
    let manifest_sha256 = header.manifest_sha256.clone();

    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(
        PREFIX_BYTES + header_bytes_len(path, limits)?,
    ))?;

    for entry in &header.manifest.entries {
        if cancel.load(Ordering::SeqCst) {
            return Err(BackupError::Storage);
        }
        let destination = match sink {
            Sink::Verify => None,
            Sink::Directory(directory) => Some(entry_path(directory, &entry.name)?),
        };
        let mut writer = match &destination {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                Some(std::fs::File::create(path)?)
            }
            None => None,
        };
        let mut hasher = Sha256::new();
        let mut written = 0u64;
        let mut index = 0u64;
        loop {
            if cancel.load(Ordering::SeqCst) {
                return Err(BackupError::Storage);
            }
            let remaining = entry.bytes - written;
            if remaining == 0 && index > 0 {
                break;
            }
            let expected_plain = remaining.min(CHUNK_BYTES as u64) as u32;
            let mut length = [0u8; 4];
            file.read_exact(&mut length)
                .map_err(|_| BackupError::TruncatedContainer)?;
            let record_len = u32::from_le_bytes(length) as u64;
            let expected_record = 1 + NONCE_BYTES as u64 + u64::from(expected_plain) + 16;
            if record_len != expected_record {
                // A container that declares a different chunk size than the
                // manifest implies is not read "as far as it goes".
                return Err(BackupError::ContentMismatch);
            }
            let mut record = vec![0u8; record_len as usize];
            file.read_exact(&mut record)
                .map_err(|_| BackupError::TruncatedContainer)?;
            let last = written + u64::from(expected_plain) >= entry.bytes;
            let context = chunk_context(&manifest_sha256, &entry.name, index, last, expected_plain);
            let plaintext = decrypt(
                &key,
                &context,
                &crate::sync::crypto::EncryptedRecord::decode(&record)?,
            )?;
            if plaintext.len() as u64 != u64::from(expected_plain) {
                return Err(BackupError::ContentMismatch);
            }
            hasher.update(&plaintext);
            if let Some(writer) = writer.as_mut() {
                writer.write_all(&plaintext)?;
            }
            written += plaintext.len() as u64;
            index += 1;
            if last {
                break;
            }
        }
        if written != entry.bytes {
            return Err(BackupError::ContentMismatch);
        }
        let computed = to_hex(&hasher.finalize());
        if computed != entry.sha256 {
            // The manifest is authenticated, so this means the payload was
            // written by something else, or damaged after the fact.
            return Err(BackupError::ContentMismatch);
        }
        if let Some(writer) = writer.as_mut() {
            writer.flush()?;
            writer.sync_all()?;
        }
    }

    // Extra bytes are not "probably nothing": a container that has content the
    // manifest does not describe is refused.
    let end = file.stream_position()?;
    let file_bytes = file.metadata()?.len();
    if end != file_bytes {
        return Err(BackupError::TruncatedContainer);
    }

    Ok(ContainerSummary {
        format_version: header.format_version,
        created_at: header.created_at,
        app_version: header.app_version,
        entries: header.manifest.entries,
        total_bytes: header.manifest.total_bytes,
    })
}

fn header_bytes_len(path: &Path, limits: &Limits) -> Result<u64, BackupError> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(10))?;
    let mut length = [0u8; 4];
    file.read_exact(&mut length)?;
    let header_len = u32::from_le_bytes(length);
    if header_len == 0 || header_len > limits.max_header_bytes {
        return Err(BackupError::TooLarge);
    }
    Ok(u64::from(header_len))
}

/// The path one logical name maps to under `directory`.
///
/// The name has already passed [`is_safe_entry_name`], and this function
/// re-checks the result: a component name can never escape the staging
/// directory, not even through a character this build did not think of.
pub fn entry_path(directory: &Path, name: &str) -> Result<PathBuf, BackupError> {
    if !is_safe_entry_name(name) {
        return Err(BackupError::UnsafeEntryName);
    }
    let mut path = directory.to_path_buf();
    for part in name.split('/') {
        path.push(part);
    }
    Ok(path)
}

/// Checks a container completely, writing nothing.
pub fn verify(
    path: &Path,
    password: &[u8],
    cancel: &AtomicBool,
    limits: &Limits,
) -> Result<ContainerSummary, BackupError> {
    walk(path, password, Sink::Verify, cancel, limits)
}

/// Checks a container and writes every entry under `directory`.
pub fn extract(
    path: &Path,
    password: &[u8],
    directory: &Path,
    cancel: &AtomicBool,
    limits: &Limits,
) -> Result<ContainerSummary, BackupError> {
    std::fs::create_dir_all(directory)?;
    walk(path, password, Sink::Directory(directory), cancel, limits)
}
