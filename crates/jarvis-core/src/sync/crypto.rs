//! Versioned local encryption, Windows key protection, and portable key backup.
//!
//! Key hierarchy:
//!
//! ```text
//! master password -> Argon2id -> password-derived KEK
//!                                -> unwraps the random master key
//!                                   -> encrypts individual records
//! ```
//!
//! The master password is never used directly as a record-encryption key. Every
//! record gets a fresh random nonce, an explicit format version, and
//! caller-supplied authenticated metadata. No custom cryptography is
//! implemented here: Argon2id and XChaCha20-Poly1305 come from audited crates.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::{CryptoProvider, EncryptedPayload, PayloadContext, SyncError};

/// Length of the random master key.
pub const KEY_BYTES: usize = 32;
/// Length of the random Argon2id salt.
pub const SALT_BYTES: usize = 16;
/// Length of an XChaCha20-Poly1305 nonce.
pub const NONCE_BYTES: usize = 24;
/// Version of the encrypted-record envelope produced by [`encrypt`].
pub const RECORD_FORMAT_VERSION: u8 = 1;
/// Version of the portable backup envelope produced by [`export_backup`].
pub const BACKUP_FORMAT_VERSION: u8 = 1;
/// Shortest accepted master password, in bytes.
pub const MIN_PASSWORD_BYTES: usize = 8;

const KDF_IDENTIFIER: &str = "argon2id";
const AEAD_IDENTIFIER: &str = "xchacha20poly1305";
const KDF_MEMORY_KIB: u32 = 19 * 1024;
const KDF_ITERATIONS: u32 = 2;
const KDF_PARALLELISM: u32 = 1;
const RECORD_AAD_DOMAIN: &[u8] = b"jarvis-local-record";
const WRAP_AAD_DOMAIN: &[u8] = b"jarvis-key-wrap";
const BACKUP_METADATA_DOMAIN: &str = "jarvis-key-backup";

// Untrusted envelopes carry their own KDF parameters, so restore bounds them to
// avoid unbounded memory or CPU use during import.
const MIN_KDF_MEMORY_KIB: u32 = 8 * 1024;
const MAX_KDF_MEMORY_KIB: u32 = 1024 * 1024;
const MAX_KDF_ITERATIONS: u32 = 16;
const MAX_KDF_PARALLELISM: u32 = 8;

/// Hidden authenticated-metadata prefix for record payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordDomain {
    Plain,
    KeyWrap,
}

impl RecordDomain {
    fn label(self) -> &'static [u8] {
        match self {
            Self::Plain => RECORD_AAD_DOMAIN,
            Self::KeyWrap => WRAP_AAD_DOMAIN,
        }
    }
}

/// A random 256-bit master key.
///
/// The key is zeroized on drop and is never rendered by `Debug`.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; KEY_BYTES]);

impl MasterKey {
    /// Wraps key material obtained elsewhere, for example from a backup import.
    pub fn from_bytes(bytes: [u8; KEY_BYTES]) -> Self {
        Self(bytes)
    }

    /// Borrows the raw key bytes for the local cipher.
    pub(crate) fn as_array(&self) -> &[u8; KEY_BYTES] {
        &self.0
    }
}

impl core::fmt::Debug for MasterKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("MasterKey(<redacted>)")
    }
}

/// An encrypted record: version tag, unique nonce, and authenticated ciphertext.
///
/// `Debug` never reveals the ciphertext or the attached metadata.
#[derive(Clone, Eq, PartialEq)]
pub struct EncryptedRecord {
    pub format_version: u8,
    pub nonce: [u8; NONCE_BYTES],
    pub ciphertext: Vec<u8>,
}

impl EncryptedRecord {
    /// `format_version` byte followed by the nonce.
    pub const HEADER_BYTES: usize = 1 + NONCE_BYTES;

    /// Serializes the envelope for opaque storage (for example a SQLite BLOB).
    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(Self::HEADER_BYTES + self.ciphertext.len());
        bytes.push(self.format_version);
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.ciphertext);
        bytes
    }

    /// Parses an envelope produced by [`EncryptedRecord::encode`].
    pub fn decode(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() <= Self::HEADER_BYTES {
            return Err(CryptoError::UnsupportedFormat);
        }
        let format_version = bytes[0];
        if format_version != RECORD_FORMAT_VERSION {
            return Err(CryptoError::UnsupportedFormat);
        }
        let mut nonce = [0; NONCE_BYTES];
        nonce.copy_from_slice(&bytes[1..Self::HEADER_BYTES]);
        Ok(Self {
            format_version,
            nonce,
            ciphertext: bytes[Self::HEADER_BYTES..].to_vec(),
        })
    }
}

impl core::fmt::Debug for EncryptedRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EncryptedRecord")
            .field("format_version", &self.format_version)
            .field("ciphertext_len", &self.ciphertext.len())
            .field("ciphertext", &"<redacted>")
            .finish()
    }
}

/// A DPAPI-protected copy of the master key.
///
/// This is deliberately a distinct type: a DPAPI blob is bound to the current
/// Windows user on the current machine and is **not** a portable backup. Only
/// [`PortableKeyBackup`] can move a key between computers.
#[derive(Clone, Eq, PartialEq)]
pub struct DpapiProtectedKey(Vec<u8>);

impl DpapiProtectedKey {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl core::fmt::Debug for DpapiProtectedKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DpapiProtectedKey")
            .field("len", &self.0.len())
            .field("bytes", &"<redacted>")
            .finish()
    }
}

/// Controlled crypto failures. Variants never carry key material or plaintext.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CryptoError {
    /// Wrong password, wrong key, or tampered ciphertext/authenticated metadata.
    InvalidPasswordOrCorruptData,
    /// Unknown envelope version or unsupported algorithm identifier.
    UnsupportedFormat,
    /// Password shorter than [`MIN_PASSWORD_BYTES`].
    WeakMasterPassword,
    /// System randomness is unavailable.
    RandomnessUnavailable,
    /// Platform key protection does not exist on this operating system.
    UnsupportedPlatform,
    /// Platform key protection failed; `os_error` is a Windows error code.
    PlatformProtectionUnavailable { os_error: u32 },
    /// The backup envelope could not be encoded or decoded.
    BackupEncodingUnavailable,
}

impl core::fmt::Display for CryptoError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPasswordOrCorruptData => {
                formatter.write_str("key material cannot be decrypted")
            }
            Self::UnsupportedFormat => formatter.write_str("unsupported encrypted storage format"),
            Self::WeakMasterPassword => {
                formatter.write_str("master password is shorter than the required minimum")
            }
            Self::RandomnessUnavailable => formatter.write_str("secure randomness unavailable"),
            Self::UnsupportedPlatform => {
                formatter.write_str("platform key protection is unavailable on this operating system")
            }
            Self::PlatformProtectionUnavailable { os_error } => write!(
                formatter,
                "platform key protection failed (windows error {os_error})"
            ),
            Self::BackupEncodingUnavailable => {
                formatter.write_str("portable backup envelope cannot be encoded")
            }
        }
    }
}

impl std::error::Error for CryptoError {}

/// Maps a crypto failure onto the sync layer without leaking details.
pub fn map_crypto_error(error: CryptoError) -> SyncError {
    match error {
        CryptoError::UnsupportedPlatform
        | CryptoError::PlatformProtectionUnavailable { .. }
        | CryptoError::RandomnessUnavailable
        | CryptoError::BackupEncodingUnavailable => SyncError::CryptoUnavailable,
        CryptoError::InvalidPasswordOrCorruptData
        | CryptoError::UnsupportedFormat
        | CryptoError::WeakMasterPassword => SyncError::CryptoRejected,
    }
}

/// Portable, versioned master-key backup envelope.
///
/// It carries its own KDF/AEAD identifiers and parameters, a random salt, a
/// random nonce, the wrapped master key, and authenticated metadata. It never
/// contains the DPAPI blob and never contains a plaintext key.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct PortableKeyBackup {
    pub format_version: u8,
    pub kdf: String,
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
    pub salt: Vec<u8>,
    pub aead: String,
    pub nonce: Vec<u8>,
    pub encrypted_master_key: Vec<u8>,
    /// Authenticated metadata; any change makes decryption fail.
    pub metadata: Vec<u8>,
}

impl PortableKeyBackup {
    /// Encodes the envelope as JSON with binary fields as byte arrays.
    pub fn to_json(&self) -> Result<String, CryptoError> {
        serde_json::to_string(self).map_err(|_| CryptoError::BackupEncodingUnavailable)
    }

    /// Parses an envelope produced by [`PortableKeyBackup::to_json`].
    pub fn from_json(text: &str) -> Result<Self, CryptoError> {
        serde_json::from_str(text).map_err(|_| CryptoError::BackupEncodingUnavailable)
    }
}

impl core::fmt::Debug for PortableKeyBackup {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PortableKeyBackup")
            .field("format_version", &self.format_version)
            .field("kdf", &self.kdf)
            .field("memory_kib", &self.memory_kib)
            .field("iterations", &self.iterations)
            .field("parallelism", &self.parallelism)
            .field("aead", &self.aead)
            .field("encrypted_master_key", &"<redacted>")
            .field("metadata", &"<redacted>")
            .finish()
    }
}

fn cipher(key: &MasterKey) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new(key.as_array().into())
}

fn random<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut value = [0; N];
    getrandom::fill(&mut value).map_err(|_| CryptoError::RandomnessUnavailable)?;
    Ok(value)
}

/// Authenticated metadata for a record: domain, format version, caller context.
fn record_aad(domain: RecordDomain, format_version: u8, context: &[u8]) -> Vec<u8> {
    let label = domain.label();
    let mut aad = Vec::with_capacity(label.len() + context.len() + 2);
    aad.extend_from_slice(label);
    aad.push(format_version);
    aad.extend_from_slice(context);
    aad
}

fn derive(
    password: &[u8],
    salt: &[u8],
    memory: u32,
    iterations: u32,
    parallelism: u32,
) -> Result<MasterKey, CryptoError> {
    if password.len() < MIN_PASSWORD_BYTES {
        return Err(CryptoError::WeakMasterPassword);
    }
    if salt.len() != SALT_BYTES {
        return Err(CryptoError::InvalidPasswordOrCorruptData);
    }
    let params = Params::new(memory, iterations, parallelism, Some(KEY_BYTES))
        .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)?;
    let mut key = [0; KEY_BYTES];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, &mut key)
        .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)?;
    Ok(MasterKey(key))
}

/// Derives the password-derived KEK with the current default parameters.
pub fn derive_wrapping_key(
    password: &[u8],
    salt: &[u8; SALT_BYTES],
) -> Result<MasterKey, CryptoError> {
    derive(password, salt, KDF_MEMORY_KIB, KDF_ITERATIONS, KDF_PARALLELISM)
}

/// Generates a fresh random master key.
pub fn random_master_key() -> Result<MasterKey, CryptoError> {
    Ok(MasterKey(random()?))
}

/// Encrypts `plaintext` with a fresh nonce and authenticated `context`.
///
/// `context` binds the ciphertext to its logical location (for example entity
/// type and entity identifier), so a payload cannot be replayed elsewhere.
pub fn encrypt(
    key: &MasterKey,
    context: &[u8],
    plaintext: &[u8],
) -> Result<EncryptedRecord, CryptoError> {
    let nonce = random::<NONCE_BYTES>()?;
    let aad = record_aad(RecordDomain::Plain, RECORD_FORMAT_VERSION, context);
    let ciphertext = cipher(key)
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)?;
    Ok(EncryptedRecord {
        format_version: RECORD_FORMAT_VERSION,
        nonce,
        ciphertext,
    })
}

/// Decrypts a record, verifying the version, nonce, ciphertext, and context.
pub fn decrypt(
    key: &MasterKey,
    context: &[u8],
    record: &EncryptedRecord,
) -> Result<Vec<u8>, CryptoError> {
    if record.format_version != RECORD_FORMAT_VERSION {
        return Err(CryptoError::UnsupportedFormat);
    }
    let aad = record_aad(RecordDomain::Plain, record.format_version, context);
    cipher(key)
        .decrypt(
            &XNonce::from(record.nonce),
            Payload {
                msg: record.ciphertext.as_ref(),
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)
}

/// A [`CryptoProvider`] backed by the real master key.
///
/// This is the only production implementation: it encrypts with
/// XChaCha20-Poly1305 and authenticates the entity context, so anything that
/// reaches synchronization storage is ciphertext.
pub struct MasterKeyCryptoProvider {
    key: MasterKey,
}

impl MasterKeyCryptoProvider {
    pub fn new(key: MasterKey) -> Self {
        Self { key }
    }
}

impl core::fmt::Debug for MasterKeyCryptoProvider {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("MasterKeyCryptoProvider(<redacted>)")
    }
}

impl CryptoProvider for MasterKeyCryptoProvider {
    fn encrypt(
        &self,
        context: &PayloadContext,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, SyncError> {
        let record = encrypt(&self.key, &context.aad(), plaintext).map_err(map_crypto_error)?;
        Ok(EncryptedPayload::from_opaque_bytes(record.encode()))
    }

    fn decrypt(
        &self,
        context: &PayloadContext,
        payload: &EncryptedPayload,
    ) -> Result<Vec<u8>, SyncError> {
        let record = EncryptedRecord::decode(payload.as_opaque_bytes()).map_err(map_crypto_error)?;
        decrypt(&self.key, &context.aad(), &record).map_err(map_crypto_error)
    }
}

fn backup_metadata(
    format_version: u8,
    kdf: &str,
    memory: u32,
    iterations: u32,
    parallelism: u32,
    salt: &[u8],
    aead: &str,
) -> Vec<u8> {
    let mut metadata = format!(
        "{BACKUP_METADATA_DOMAIN}|{format_version}|{kdf}|{memory}|{iterations}|{parallelism}|{aead}|"
    )
    .into_bytes();
    metadata.extend_from_slice(salt);
    metadata
}

/// Wraps the master key into a portable, password-protected backup envelope.
pub fn export_backup(
    master_key: &MasterKey,
    password: &[u8],
) -> Result<PortableKeyBackup, CryptoError> {
    let salt = random::<SALT_BYTES>()?;
    let wrapping = derive(password, &salt, KDF_MEMORY_KIB, KDF_ITERATIONS, KDF_PARALLELISM)?;
    let nonce = random::<NONCE_BYTES>()?;
    let metadata = backup_metadata(
        BACKUP_FORMAT_VERSION,
        KDF_IDENTIFIER,
        KDF_MEMORY_KIB,
        KDF_ITERATIONS,
        KDF_PARALLELISM,
        &salt,
        AEAD_IDENTIFIER,
    );
    let aad = record_aad(RecordDomain::KeyWrap, BACKUP_FORMAT_VERSION, &metadata);
    let encrypted_master_key = cipher(&wrapping)
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: master_key.as_array(),
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)?;
    Ok(PortableKeyBackup {
        format_version: BACKUP_FORMAT_VERSION,
        kdf: KDF_IDENTIFIER.to_string(),
        memory_kib: KDF_MEMORY_KIB,
        iterations: KDF_ITERATIONS,
        parallelism: KDF_PARALLELISM,
        salt: salt.to_vec(),
        aead: AEAD_IDENTIFIER.to_string(),
        nonce: nonce.to_vec(),
        encrypted_master_key,
        metadata,
    })
}

/// Restores the master key from a portable backup with the original password.
///
/// Wrong passwords, tampered metadata or ciphertext, unknown versions, and
/// out-of-range KDF parameters are all rejected.
pub fn import_backup(
    backup: &PortableKeyBackup,
    password: &[u8],
) -> Result<MasterKey, CryptoError> {
    if backup.format_version != BACKUP_FORMAT_VERSION
        || backup.kdf != KDF_IDENTIFIER
        || backup.aead != AEAD_IDENTIFIER
        || backup.nonce.len() != NONCE_BYTES
        || backup.salt.len() != SALT_BYTES
    {
        // Unknown or unsupported envelope: never attempt a best-effort decode.
        return Err(CryptoError::UnsupportedFormat);
    }
    if !(MIN_KDF_MEMORY_KIB..=MAX_KDF_MEMORY_KIB).contains(&backup.memory_kib)
        || backup.iterations == 0
        || backup.iterations > MAX_KDF_ITERATIONS
        || backup.parallelism == 0
        || backup.parallelism > MAX_KDF_PARALLELISM
    {
        return Err(CryptoError::UnsupportedFormat);
    }
    let expected = backup_metadata(
        backup.format_version,
        &backup.kdf,
        backup.memory_kib,
        backup.iterations,
        backup.parallelism,
        &backup.salt,
        &backup.aead,
    );
    if expected != backup.metadata {
        return Err(CryptoError::InvalidPasswordOrCorruptData);
    }
    let wrapping = derive(
        password,
        &backup.salt,
        backup.memory_kib,
        backup.iterations,
        backup.parallelism,
    )?;
    let mut nonce = [0; NONCE_BYTES];
    nonce.copy_from_slice(&backup.nonce);
    let aad = record_aad(RecordDomain::KeyWrap, backup.format_version, &backup.metadata);
    let mut plaintext = cipher(&wrapping)
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &backup.encrypted_master_key,
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)?;
    if plaintext.len() != KEY_BYTES {
        plaintext.zeroize();
        return Err(CryptoError::InvalidPasswordOrCorruptData);
    }
    let mut key = [0; KEY_BYTES];
    key.copy_from_slice(&plaintext);
    plaintext.zeroize();
    Ok(MasterKey(key))
}

/// Protects the master key for the current Windows user via DPAPI.
#[cfg(windows)]
pub fn dpapi_protect(master_key: &MasterKey) -> Result<DpapiProtectedKey, CryptoError> {
    Ok(DpapiProtectedKey(dpapi_call(master_key.as_array(), true)?))
}

/// Recovers a DPAPI-protected master key for the current Windows user.
#[cfg(windows)]
pub fn dpapi_unprotect(blob: &DpapiProtectedKey) -> Result<MasterKey, CryptoError> {
    let data = dpapi_call(blob.as_bytes(), false)?;
    if data.len() != KEY_BYTES {
        return Err(CryptoError::InvalidPasswordOrCorruptData);
    }
    let mut key = [0; KEY_BYTES];
    key.copy_from_slice(&data);
    Ok(MasterKey(key))
}

#[cfg(not(windows))]
pub fn dpapi_protect(_: &MasterKey) -> Result<DpapiProtectedKey, CryptoError> {
    Err(CryptoError::UnsupportedPlatform)
}

#[cfg(not(windows))]
pub fn dpapi_unprotect(_: &DpapiProtectedKey) -> Result<MasterKey, CryptoError> {
    Err(CryptoError::UnsupportedPlatform)
}

#[cfg(windows)]
fn dpapi_call(input: &[u8], protect: bool) -> Result<Vec<u8>, CryptoError> {
    use windows_sys::Win32::{
        Foundation::{GetLastError, LocalFree},
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    if input.is_empty() || input.len() > u32::MAX as usize {
        return Err(CryptoError::PlatformProtectionUnavailable { os_error: 0 });
    }
    let source = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: core::ptr::null_mut(),
    };
    // `CRYPTPROTECT_UI_FORBIDDEN` keeps DPAPI from showing any system prompt.
    // The description, optional entropy, and prompt struct stay null; scope is
    // the current user because no `CRYPTPROTECT_LOCAL_MACHINE` is requested.
    let succeeded = unsafe {
        if protect {
            CryptProtectData(
                &source,
                core::ptr::null(),
                core::ptr::null(),
                core::ptr::null_mut(),
                core::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &source,
                core::ptr::null_mut(),
                core::ptr::null(),
                core::ptr::null_mut(),
                core::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if succeeded == 0 {
        let os_error = unsafe { GetLastError() };
        return Err(CryptoError::PlatformProtectionUnavailable { os_error });
    }
    if output.pbData.is_null() {
        return Err(CryptoError::PlatformProtectionUnavailable { os_error: 0 });
    }
    // Copy out, then release the buffer Windows allocated for us.
    let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let result = bytes.to_vec();
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_envelope_round_trips_and_rejects_unknown_versions() {
        let key = random_master_key().unwrap();
        let record = encrypt(&key, b"context", b"fictional secret").unwrap();
        let encoded = record.encode();
        let decoded = EncryptedRecord::decode(&encoded).unwrap();
        assert_eq!(decoded, record);
        assert_eq!(
            decrypt(&key, b"context", &decoded).unwrap(),
            b"fictional secret"
        );

        let mut unknown = encoded.clone();
        unknown[0] = RECORD_FORMAT_VERSION + 1;
        assert_eq!(
            EncryptedRecord::decode(&unknown),
            Err(CryptoError::UnsupportedFormat)
        );
        assert_eq!(
            EncryptedRecord::decode(&encoded[..EncryptedRecord::HEADER_BYTES]),
            Err(CryptoError::UnsupportedFormat)
        );
    }

    #[test]
    fn context_change_is_detected_and_nonces_are_unique() {
        let key = random_master_key().unwrap();
        let first = encrypt(&key, b"entity-a", b"fictional note").unwrap();
        let second = encrypt(&key, b"entity-a", b"fictional note").unwrap();
        assert_ne!(first.nonce, second.nonce);
        assert_ne!(first.ciphertext, second.ciphertext);
        assert_eq!(
            decrypt(&key, b"entity-b", &first),
            Err(CryptoError::InvalidPasswordOrCorruptData)
        );
    }

    #[test]
    fn weak_master_passwords_are_rejected() {
        let key = random_master_key().unwrap();
        assert_eq!(
            export_backup(&key, b"short"),
            Err(CryptoError::WeakMasterPassword)
        );
    }

    #[test]
    fn debug_output_never_reveals_keys_or_plaintext() {
        let key = random_master_key().unwrap();
        let record = encrypt(&key, b"entity-a", b"FICTIONAL_SECRET_PLAINTEXT").unwrap();
        let backup = export_backup(&key, b"fixture-password").unwrap();
        let mut rendered = format!("{key:?}{record:?}{backup:?}");
        rendered.push_str(&format!("{:?}", MasterKeyCryptoProvider::new(random_master_key().unwrap())));
        assert!(!rendered.contains("FICTIONAL_SECRET_PLAINTEXT"));
        assert!(!rendered.contains("fixture-password"));
        assert!(rendered.contains("<redacted>"));
    }
}
