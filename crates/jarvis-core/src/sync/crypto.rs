//! Versioned local encryption, Windows key protection, and portable key backup.
//!
//! Key hierarchy:
//!
//! ```text
//! master password -> Argon2id -> password-derived KEK
//!                                -> unwraps the random master key
//!                                   -> HKDF-SHA256 per purpose
//!                                      -> encrypts the records of that purpose
//! ```
//!
//! The master password is never used directly as a record-encryption key, and no
//! two features share a working key: the notes, the password vault, and (later)
//! AI memory each derive their own key from the master key with a distinct HKDF
//! domain label. Every record gets a fresh random nonce, an explicit format
//! version, and caller-supplied authenticated metadata. No custom cryptography
//! is implemented here: Argon2id, HKDF-SHA256, and XChaCha20-Poly1305 all come
//! from audited RustCrypto crates.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use super::{CryptoProvider, EncryptedPayload, PayloadContext, SyncError};

/// Length of the random master key.
pub const KEY_BYTES: usize = 32;
/// Length of the random Argon2id salt.
pub const SALT_BYTES: usize = 16;
/// Length of an XChaCha20-Poly1305 nonce.
pub const NONCE_BYTES: usize = 24;
/// Version of the encrypted-record envelope written by [`encrypt`].
///
/// Version 2 records are encrypted with a purpose-derived key.
pub const RECORD_FORMAT_VERSION: u8 = 2;
/// Version 1 records were encrypted with the raw master key, before key
/// separation. They are still readable so existing notes survive the change;
/// every write upgrades to the current version.
pub const RECORD_FORMAT_VERSION_LEGACY: u8 = 1;
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

/// Fixed application salt for purpose-key derivation.
///
/// The input key material is already a 256-bit random key, so this salt exists to
/// keep the derivation bound to this application rather than to add work factor.
const PURPOSE_KDF_SALT: &[u8] = b"jarvis-local-storage-purpose-v1";

// Untrusted envelopes carry their own KDF parameters, so restore bounds them to
// avoid unbounded memory or CPU use during import.
const MIN_KDF_MEMORY_KIB: u32 = 8 * 1024;
const MAX_KDF_MEMORY_KIB: u32 = 1024 * 1024;
const MAX_KDF_ITERATIONS: u32 = 16;
const MAX_KDF_PARALLELISM: u32 = 8;

/// Which feature a derived key belongs to.
///
/// The label is part of the on-disk contract: changing one would make existing
/// records undecryptable, so labels are versioned and must never be reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyPurpose {
    Notes,
    Vault,
    AiMemory,
    /// The user's spelling dictionary: names and internal terms the user taught.
    Autocorrect,
}

impl KeyPurpose {
    /// HKDF `info` label. Stable for the lifetime of the format.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Notes => "JARVIS/notes/v1",
            Self::Vault => "JARVIS/vault/v1",
            Self::AiMemory => "JARVIS/ai-memory/v1",
            Self::Autocorrect => "JARVIS/autocorrect/v1",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Notes => "notes",
            Self::Vault => "vault",
            Self::AiMemory => "ai-memory",
            Self::Autocorrect => "autocorrect",
        }
    }

    /// Every purpose this build knows how to derive.
    pub fn all() -> [Self; 4] {
        [
            Self::Notes,
            Self::Vault,
            Self::AiMemory,
            Self::Autocorrect,
        ]
    }
}

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
    ///
    /// Both the current version and the legacy version are accepted; unknown
    /// versions are refused before any decryption is attempted.
    pub fn decode(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() <= Self::HEADER_BYTES {
            return Err(CryptoError::UnsupportedFormat);
        }
        let format_version = bytes[0];
        if !is_supported_record_version(format_version) {
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

/// Whether a record envelope version may be decrypted by this build.
pub fn is_supported_record_version(version: u8) -> bool {
    version == RECORD_FORMAT_VERSION || version == RECORD_FORMAT_VERSION_LEGACY
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
    /// The new master password equals the current one.
    MasterPasswordUnchanged,
    /// System randomness is unavailable.
    RandomnessUnavailable,
    /// Key derivation failed.
    KeyDerivationUnavailable,
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
            Self::MasterPasswordUnchanged => {
                formatter.write_str("the new master password matches the current one")
            }
            Self::RandomnessUnavailable => formatter.write_str("secure randomness unavailable"),
            Self::KeyDerivationUnavailable => formatter.write_str("key derivation failed"),
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
        | CryptoError::KeyDerivationUnavailable
        | CryptoError::BackupEncodingUnavailable => SyncError::CryptoUnavailable,
        CryptoError::InvalidPasswordOrCorruptData
        | CryptoError::UnsupportedFormat
        | CryptoError::WeakMasterPassword
        | CryptoError::MasterPasswordUnchanged => SyncError::CryptoRejected,
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

/// Derives the working key of one feature from the master key.
///
/// This is HKDF-SHA256 (RFC 5869) with a fixed application salt and a versioned
/// domain label, so no feature ever uses the raw master key and no two features
/// share a key. Nothing derived here is written to disk: purpose keys exist only
/// while the storage is unlocked.
pub fn derive_purpose_key(
    master_key: &MasterKey,
    purpose: KeyPurpose,
) -> Result<MasterKey, CryptoError> {
    let hkdf = Hkdf::<Sha256>::new(Some(PURPOSE_KDF_SALT), master_key.as_array());
    let mut output = Zeroizing::new([0u8; KEY_BYTES]);
    hkdf.expand(purpose.label().as_bytes(), output.as_mut())
        .map_err(|_| CryptoError::KeyDerivationUnavailable)?;
    Ok(MasterKey(*output))
}

/// Encrypts `plaintext` with a fresh nonce and authenticated `context`.
///
/// `context` binds the ciphertext to its logical location (for example entity
/// type and entity identifier), so a payload cannot be replayed elsewhere.
/// `key` must be the purpose key of the feature that owns the record.
pub fn encrypt(
    key: &MasterKey,
    context: &[u8],
    plaintext: &[u8],
) -> Result<EncryptedRecord, CryptoError> {
    encrypt_versioned(key, context, plaintext, RECORD_FORMAT_VERSION)
}

/// Encrypts with an explicit envelope version.
///
/// Production code always writes [`RECORD_FORMAT_VERSION`]; the legacy version
/// exists so this module can produce and verify pre-separation fixtures.
pub fn encrypt_versioned(
    key: &MasterKey,
    context: &[u8],
    plaintext: &[u8],
    format_version: u8,
) -> Result<EncryptedRecord, CryptoError> {
    if !is_supported_record_version(format_version) {
        return Err(CryptoError::UnsupportedFormat);
    }
    let nonce = random::<NONCE_BYTES>()?;
    let aad = record_aad(RecordDomain::Plain, format_version, context);
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
        format_version,
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
    if !is_supported_record_version(record.format_version) {
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

/// A [`CryptoProvider`] for the notes domain, backed by the master key.
///
/// It encrypts with the notes purpose key and additionally keeps the master key
/// so that notes written before key separation (format version 1) still decrypt.
/// Every write uses the purpose key. Because it holds the master key, only the
/// notes storage may use this type; the password vault uses
/// [`PurposeKeyProvider`], which never sees the master key at all.
pub struct MasterKeyCryptoProvider {
    master_key: MasterKey,
    purpose_key: MasterKey,
}

impl MasterKeyCryptoProvider {
    /// Notes-purpose provider, which is also the historical behavior.
    pub fn new(master_key: MasterKey) -> Self {
        let purpose_key = derive_purpose_key(&master_key, KeyPurpose::Notes)
            .expect("HKDF output length is fixed and always valid");
        Self {
            master_key,
            purpose_key,
        }
    }

    /// Borrows the master key so it can be re-wrapped into a new portable
    /// backup envelope or protected with DPAPI. The key is never rendered or
    /// copied out.
    pub fn key(&self) -> &MasterKey {
        &self.master_key
    }

    /// The domain this provider writes.
    pub fn purpose(&self) -> KeyPurpose {
        KeyPurpose::Notes
    }
}

impl core::fmt::Debug for MasterKeyCryptoProvider {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("MasterKeyCryptoProvider")
            .field("purpose", &KeyPurpose::Notes)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl CryptoProvider for MasterKeyCryptoProvider {
    fn encrypt(
        &self,
        context: &PayloadContext,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, SyncError> {
        let record =
            encrypt(&self.purpose_key, &context.aad(), plaintext).map_err(map_crypto_error)?;
        Ok(EncryptedPayload::from_opaque_bytes(record.encode()))
    }

    fn decrypt(
        &self,
        context: &PayloadContext,
        payload: &EncryptedPayload,
    ) -> Result<Vec<u8>, SyncError> {
        let record = EncryptedRecord::decode(payload.as_opaque_bytes()).map_err(map_crypto_error)?;
        // Version 1 records predate key separation and were written with the raw
        // master key; the version byte is authenticated, so this choice cannot be
        // confused by tampering.
        let key = if record.format_version == RECORD_FORMAT_VERSION_LEGACY {
            &self.master_key
        } else {
            &self.purpose_key
        };
        decrypt(key, &context.aad(), &record).map_err(map_crypto_error)
    }
}

/// A [`CryptoProvider`] that holds only one derived purpose key.
///
/// This is the type the password vault uses. It has no accessor for the master
/// key and no legacy fallback, so the domain separation is enforced by the type
/// system rather than by convention: a provider built for one purpose cannot
/// read another purpose's records even if it is handed their ciphertext.
pub struct PurposeKeyProvider {
    purpose_key: MasterKey,
    purpose: KeyPurpose,
}

impl PurposeKeyProvider {
    /// Derives the provider key from the master key. The master key itself is
    /// neither stored nor copied into the provider.
    pub fn derive(master_key: &MasterKey, purpose: KeyPurpose) -> Result<Self, CryptoError> {
        Ok(Self {
            purpose_key: derive_purpose_key(master_key, purpose)?,
            purpose,
        })
    }

    pub fn purpose(&self) -> KeyPurpose {
        self.purpose
    }
}

impl core::fmt::Debug for PurposeKeyProvider {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("PurposeKeyProvider")
            .field("purpose", &self.purpose)
            .field("key", &"<redacted>")
            .finish()
    }
}

impl CryptoProvider for PurposeKeyProvider {
    fn encrypt(
        &self,
        context: &PayloadContext,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, SyncError> {
        let record =
            encrypt(&self.purpose_key, &context.aad(), plaintext).map_err(map_crypto_error)?;
        Ok(EncryptedPayload::from_opaque_bytes(record.encode()))
    }

    fn decrypt(
        &self,
        context: &PayloadContext,
        payload: &EncryptedPayload,
    ) -> Result<Vec<u8>, SyncError> {
        let record = EncryptedRecord::decode(payload.as_opaque_bytes()).map_err(map_crypto_error)?;
        if record.format_version != RECORD_FORMAT_VERSION {
            // No master key is kept, so pre-separation records are out of reach
            // for this domain by construction.
            return Err(SyncError::CryptoRejected);
        }
        decrypt(&self.purpose_key, &context.aad(), &record).map_err(map_crypto_error)
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

/// Seals arbitrary bytes for the current Windows user via DPAPI.
///
/// Used by features that must keep a small payload private without needing the master key —
/// a reminder text, for instance, which has to be readable when the timer fires, including
/// while the encrypted storage is locked. The scope is the current user, the same as the
/// master-key blob, and no prompt is ever shown.
#[cfg(windows)]
pub fn dpapi_seal_bytes(plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    dpapi_call(plaintext, true)
}

/// Opens bytes sealed by [`dpapi_seal_bytes`] for the current Windows user.
#[cfg(windows)]
pub fn dpapi_open_bytes(sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
    dpapi_call(sealed, false)
}

#[cfg(not(windows))]
pub fn dpapi_seal_bytes(_: &[u8]) -> Result<Vec<u8>, CryptoError> {
    Err(CryptoError::UnsupportedPlatform)
}

#[cfg(not(windows))]
pub fn dpapi_open_bytes(_: &[u8]) -> Result<Vec<u8>, CryptoError> {
    Err(CryptoError::UnsupportedPlatform)
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

    use uuid::Uuid;

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

    #[test]
    fn purpose_keys_are_deterministic_and_domain_separated() {
        let master = random_master_key().unwrap();
        let notes = derive_purpose_key(&master, KeyPurpose::Notes).unwrap();
        let vault = derive_purpose_key(&master, KeyPurpose::Vault).unwrap();
        let memory = derive_purpose_key(&master, KeyPurpose::AiMemory).unwrap();
        let autocorrect = derive_purpose_key(&master, KeyPurpose::Autocorrect).unwrap();

        // Deterministic: unlocking again must produce the same keys.
        let notes_again = derive_purpose_key(&master, KeyPurpose::Notes).unwrap();
        assert_eq!(notes.as_array(), notes_again.as_array());

        // Distinct for every purpose, and never equal to the master key itself.
        assert_ne!(notes.as_array(), vault.as_array());
        assert_ne!(notes.as_array(), memory.as_array());
        assert_ne!(vault.as_array(), memory.as_array());
        assert_ne!(notes.as_array(), autocorrect.as_array());
        assert_ne!(vault.as_array(), autocorrect.as_array());
        assert_ne!(memory.as_array(), autocorrect.as_array());
        assert_ne!(notes.as_array(), master.as_array());

        // A different master key yields different purpose keys.
        let other = random_master_key().unwrap();
        assert_ne!(
            notes.as_array(),
            derive_purpose_key(&other, KeyPurpose::Notes)
                .unwrap()
                .as_array()
        );

        // Labels are the documented, versioned constants.
        assert_eq!(KeyPurpose::Notes.label(), "JARVIS/notes/v1");
        assert_eq!(KeyPurpose::Vault.label(), "JARVIS/vault/v1");
        assert_eq!(KeyPurpose::AiMemory.label(), "JARVIS/ai-memory/v1");
        assert_eq!(KeyPurpose::Autocorrect.label(), "JARVIS/autocorrect/v1");
        let mut labels: Vec<&str> = KeyPurpose::all().iter().map(KeyPurpose::label).collect();
        assert_eq!(labels.len(), 4);
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), 4, "every purpose needs its own label");
        for purpose in KeyPurpose::all() {
            assert!(purpose.label().starts_with("JARVIS/"));
            assert!(!purpose.as_str().is_empty());
        }
    }

    #[test]
    fn a_record_cannot_be_read_with_another_purpose_key() {
        let master = random_master_key().unwrap();
        let notes = MasterKeyCryptoProvider::new(MasterKey::from_bytes(*master.as_array()));
        let vault = PurposeKeyProvider::derive(&master, KeyPurpose::Vault).unwrap();
        let context = PayloadContext::new(super::super::SyncEntityType::VaultRecord, Uuid::new_v4());

        let payload = vault.encrypt(&context, b"FICTIONAL_VAULT_SECRET").unwrap();
        assert_eq!(
            vault.decrypt(&context, &payload).unwrap(),
            b"FICTIONAL_VAULT_SECRET"
        );
        // The notes provider must not open a vault record, even with identical
        // associated data, because it derives a different working key.
        assert_eq!(
            notes.decrypt(&context, &payload),
            Err(SyncError::CryptoRejected)
        );

        let note_context = PayloadContext::new(super::super::SyncEntityType::Note, Uuid::new_v4());
        let note_payload = notes.encrypt(&note_context, b"FICTIONAL_NOTE").unwrap();
        assert_eq!(
            vault.decrypt(&note_context, &note_payload),
            Err(SyncError::CryptoRejected)
        );

        assert_eq!(vault.purpose(), KeyPurpose::Vault);
        assert_eq!(notes.purpose(), KeyPurpose::Notes);

        // Cross-purpose ciphertext with a shared payload context still fails.
        let vault_note_context =
            PayloadContext::new(super::super::SyncEntityType::Note, note_context.entity_id);
        let shared = vault
            .encrypt(&vault_note_context, b"FICTIONAL_SHARED_CONTEXT")
            .unwrap();
        assert_eq!(
            notes.decrypt(&vault_note_context, &shared),
            Err(SyncError::CryptoRejected)
        );
    }

    #[test]
    fn records_written_before_key_separation_still_decrypt() {
        let master = random_master_key().unwrap();
        let provider = MasterKeyCryptoProvider::new(MasterKey::from_bytes(*master.as_array()));
        let context = PayloadContext::new(super::super::SyncEntityType::Note, Uuid::new_v4());
        let aad = context.aad();

        // Exactly what the previous release wrote: version 1 envelope, raw
        // master key, no domain separation.
        let legacy = encrypt_versioned(
            &master,
            &aad,
            b"FICTIONAL_NOTE_WRITTEN_BEFORE_KEY_SEPARATION",
            RECORD_FORMAT_VERSION_LEGACY,
        )
        .unwrap();
        assert_eq!(legacy.format_version, RECORD_FORMAT_VERSION_LEGACY);

        let payload = EncryptedPayload::from_opaque_bytes(legacy.encode());
        assert_eq!(
            provider.decrypt(&context, &payload).unwrap(),
            b"FICTIONAL_NOTE_WRITTEN_BEFORE_KEY_SEPARATION"
        );

        // New writes use the current version and the derived key.
        let fresh = provider.encrypt(&context, b"FICTIONAL_NOTE").unwrap();
        assert_eq!(
            EncryptedRecord::decode(fresh.as_opaque_bytes())
                .unwrap()
                .format_version,
            RECORD_FORMAT_VERSION
        );

        // A purpose provider keeps no master key, so it cannot reach legacy
        // records even when it is handed their ciphertext.
        let vault = PurposeKeyProvider::derive(&master, KeyPurpose::Vault).unwrap();
        assert_eq!(
            vault.decrypt(&context, &payload),
            Err(SyncError::CryptoRejected)
        );
    }

    #[test]
    fn unknown_record_versions_are_refused() {
        let master = random_master_key().unwrap();
        let record = encrypt(&master, b"context", b"fixture").unwrap();
        let mut encoded = record.encode();
        encoded[0] = 0;
        assert_eq!(
            EncryptedRecord::decode(&encoded),
            Err(CryptoError::UnsupportedFormat)
        );
        encoded[0] = 3;
        assert_eq!(
            EncryptedRecord::decode(&encoded),
            Err(CryptoError::UnsupportedFormat)
        );
        assert!(is_supported_record_version(RECORD_FORMAT_VERSION));
        assert!(is_supported_record_version(RECORD_FORMAT_VERSION_LEGACY));
        assert!(!is_supported_record_version(0));
    }
}
