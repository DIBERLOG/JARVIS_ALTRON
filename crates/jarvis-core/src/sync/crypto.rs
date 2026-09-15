//! Versioned, authenticated local encryption and portable key backup.
//! No password is used directly to encrypt a sync record.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const KEY_BYTES: usize = 32;
pub const SALT_BYTES: usize = 16;
const NONCE_BYTES: usize = 24;
const FORMAT_VERSION: u8 = 1;
const KDF_MEMORY_KIB: u32 = 19 * 1024;
const KDF_ITERATIONS: u32 = 2;

#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MasterKey([u8; KEY_BYTES]);
impl core::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

#[derive(Clone)]
pub struct EncryptedRecord {
    pub nonce: [u8; NONCE_BYTES],
    pub ciphertext: Vec<u8>,
}
impl core::fmt::Debug for EncryptedRecord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("EncryptedRecord(<redacted>)")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CryptoError {
    InvalidPasswordOrCorruptData,
    UnsupportedFormat,
    RandomnessUnavailable,
    UnsupportedPlatform,
    PlatformProtectionUnavailable,
}
impl core::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidPasswordOrCorruptData => "key material cannot be decrypted",
            Self::UnsupportedFormat => "unsupported encrypted storage format",
            Self::RandomnessUnavailable => "secure randomness unavailable",
            Self::UnsupportedPlatform => {
                "platform key protection is unavailable on this operating system"
            }
            Self::PlatformProtectionUnavailable => "platform key protection failed",
        })
    }
}
impl std::error::Error for CryptoError {}

#[derive(Clone, Debug, Deserialize, Serialize)]
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
    pub metadata: Vec<u8>,
}

fn cipher(key: &MasterKey) -> XChaCha20Poly1305 {
    XChaCha20Poly1305::new((&key.0).into())
}
fn random<const N: usize>() -> Result<[u8; N], CryptoError> {
    let mut value = [0; N];
    getrandom::fill(&mut value).map_err(|_| CryptoError::RandomnessUnavailable)?;
    Ok(value)
}
fn derive(
    password: &[u8],
    salt: &[u8],
    memory: u32,
    iterations: u32,
    parallelism: u32,
) -> Result<MasterKey, CryptoError> {
    if salt.len() != SALT_BYTES {
        return Err(CryptoError::InvalidPasswordOrCorruptData);
    }
    let mut key = [0; KEY_BYTES];
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(memory, iterations, parallelism, Some(KEY_BYTES))
            .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)?,
    )
    .hash_password_into(password, salt, &mut key)
    .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)?;
    Ok(MasterKey(key))
}
pub fn derive_wrapping_key(
    password: &[u8],
    salt: &[u8; SALT_BYTES],
) -> Result<MasterKey, CryptoError> {
    derive(password, salt, KDF_MEMORY_KIB, KDF_ITERATIONS, 1)
}
pub fn random_master_key() -> Result<MasterKey, CryptoError> {
    Ok(MasterKey(random()?))
}
pub fn encrypt(key: &MasterKey, plaintext: &[u8]) -> Result<EncryptedRecord, CryptoError> {
    let nonce = random()?;
    let ciphertext = cipher(key)
        .encrypt(&XNonce::from(nonce), plaintext)
        .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)?;
    Ok(EncryptedRecord { nonce, ciphertext })
}
pub fn decrypt(key: &MasterKey, value: &EncryptedRecord) -> Result<Vec<u8>, CryptoError> {
    cipher(key)
        .decrypt(&XNonce::from(value.nonce), value.ciphertext.as_ref())
        .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)
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
    format!("jarvis-key-backup|{format_version}|{kdf}|{memory}|{iterations}|{parallelism}|{aead}|")
        .into_bytes()
        .into_iter()
        .chain(salt.iter().copied())
        .collect()
}
pub fn export_backup(
    master_key: &MasterKey,
    password: &[u8],
) -> Result<PortableKeyBackup, CryptoError> {
    let salt = random::<SALT_BYTES>()?;
    let wrapping = derive(password, &salt, KDF_MEMORY_KIB, KDF_ITERATIONS, 1)?;
    let nonce = random::<NONCE_BYTES>()?;
    let metadata = backup_metadata(
        FORMAT_VERSION,
        "argon2id",
        KDF_MEMORY_KIB,
        KDF_ITERATIONS,
        1,
        &salt,
        "xchacha20poly1305",
    );
    let encrypted_master_key = cipher(&wrapping)
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &master_key.0,
                aad: &metadata,
            },
        )
        .map_err(|_| CryptoError::InvalidPasswordOrCorruptData)?;
    Ok(PortableKeyBackup {
        format_version: FORMAT_VERSION,
        kdf: "argon2id".into(),
        memory_kib: KDF_MEMORY_KIB,
        iterations: KDF_ITERATIONS,
        parallelism: 1,
        salt: salt.to_vec(),
        aead: "xchacha20poly1305".into(),
        nonce: nonce.to_vec(),
        encrypted_master_key,
        metadata,
    })
}
pub fn import_backup(
    backup: &PortableKeyBackup,
    password: &[u8],
) -> Result<MasterKey, CryptoError> {
    if backup.format_version != FORMAT_VERSION
        || backup.kdf != "argon2id"
        || backup.aead != "xchacha20poly1305"
        || backup.nonce.len() != NONCE_BYTES
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
    let mut plaintext = cipher(&wrapping)
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &backup.encrypted_master_key,
                aad: &backup.metadata,
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

#[cfg(windows)]
pub fn dpapi_protect(master_key: &MasterKey) -> Result<Vec<u8>, CryptoError> {
    dpapi(&master_key.0, true)
}
#[cfg(windows)]
pub fn dpapi_unprotect(blob: &[u8]) -> Result<MasterKey, CryptoError> {
    let data = dpapi(blob, false)?;
    if data.len() != KEY_BYTES {
        return Err(CryptoError::InvalidPasswordOrCorruptData);
    }
    let mut key = [0; KEY_BYTES];
    key.copy_from_slice(&data);
    Ok(MasterKey(key))
}
#[cfg(not(windows))]
pub fn dpapi_protect(_: &MasterKey) -> Result<Vec<u8>, CryptoError> {
    Err(CryptoError::UnsupportedPlatform)
}
#[cfg(not(windows))]
pub fn dpapi_unprotect(_: &[u8]) -> Result<MasterKey, CryptoError> {
    Err(CryptoError::UnsupportedPlatform)
}
#[cfg(windows)]
fn dpapi(input: &[u8], protect: bool) -> Result<Vec<u8>, CryptoError> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    let source = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: core::ptr::null_mut(),
    };
    let ok = unsafe {
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
    if ok == 0 {
        return Err(CryptoError::PlatformProtectionUnavailable);
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe { LocalFree(output.pbData.cast()) };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backup_round_trip_authenticates_metadata() {
        let key = random_master_key().unwrap();
        let backup = export_backup(&key, b"fixture-password").unwrap();
        assert!(format!("{key:?}{backup:?}").contains("MasterKey(<redacted>)"));
        let record = encrypt(&key, b"fictional secret").unwrap();
        let restored = import_backup(&backup, b"fixture-password").unwrap();
        assert_eq!(decrypt(&restored, &record).unwrap(), b"fictional secret");
        assert!(import_backup(&backup, b"wrong").is_err());
        let mut altered = backup.clone();
        altered.metadata.push(0);
        assert!(import_backup(&altered, b"fixture-password").is_err());
    }
}
