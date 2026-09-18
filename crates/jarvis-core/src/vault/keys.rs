//! Master-password change.
//!
//! Changing the master password re-wraps the existing master key under a new
//! Argon2id-derived KEK; the data itself is not re-encrypted, because the master
//! key does not change. Full master-key rotation (which would require re-reading
//! and re-encrypting every record) is deliberately **not** implemented yet; see
//! `VAULT.md`.
//!
//! Ordering is chosen so the old key files stay valid until the new ones are
//! completely written, and so no copy of the master key is ever written
//! unwrapped:
//!
//! ```text
//! verify the current password (fails -> nothing changed)
//! build the new envelope and the new DPAPI blob in memory (fails -> nothing changed)
//! atomically replace key.backup.json
//! atomically replace key.dpapi (Windows; skip when unsupported)
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::notes::vault::NotesVault;
use crate::sync::crypto::{dpapi_protect, MIN_PASSWORD_BYTES};

use super::model::VaultError;
use super::VaultResult;

/// Outcome of a successful master-password change.
#[derive(Clone, Debug, Serialize)]
pub struct MasterPasswordChange {
    /// The portable envelope was replaced.
    pub backup_replaced: bool,
    /// The DPAPI blob was refreshed for this Windows user.
    pub dpapi_updated: bool,
    /// Where the portable envelope lives, for display.
    pub backup_path: String,
}

/// Re-wraps the master key so `new_password` unlocks the storage.
///
/// The vault must be unlocked or the current password must be correct; the
/// current password is always verified, so a caller cannot change it without
/// knowing it.
pub fn change_master_password(
    vault: &mut NotesVault,
    current_password: &str,
    new_password: &str,
) -> VaultResult<MasterPasswordChange> {
    if new_password.len() < MIN_PASSWORD_BYTES {
        return Err(VaultError::Crypto(
            crate::sync::crypto::CryptoError::WeakMasterPassword,
        ));
    }
    if current_password == new_password {
        return Err(VaultError::Crypto(
            crate::sync::crypto::CryptoError::MasterPasswordUnchanged,
        ));
    }

    // 1. Verify the current password. A wrong password leaves everything as is.
    vault.unlock_with_password(current_password)?;

    // 2. Build both wrappers in memory before touching any file.
    let envelope_json = vault.export_backup_json(new_password)?;
    let dpapi_blob = {
        let key = vault
            .store()
            .map_err(|_| VaultError::StorageLocked)?
            .crypto()
            .key();
        dpapi_protect(key)
    };

    // 3. Replace the portable envelope first: it is the artifact that must never
    //    be lost. The write goes through a temporary file and a rename, so a
    //    failure leaves the previous envelope fully usable.
    let backup_path = vault.paths().backup_key.clone();
    write_file_atomic(&backup_path, envelope_json.as_bytes())?;

    // 4. Refresh the machine-bound copy. The DPAPI blob protects the same master
    //    key, so a failure here cannot lock the user out.
    let mut dpapi_updated = false;
    if let Ok(blob) = dpapi_blob {
        write_file_atomic(&vault.paths().dpapi_key, blob.as_bytes())?;
        dpapi_updated = true;
    }

    Ok(MasterPasswordChange {
        backup_replaced: true,
        dpapi_updated,
        backup_path: backup_path.display().to_string(),
    })
}

/// Writes through a temporary file so a crash cannot leave a truncated key.
///
/// The temporary file only ever holds ciphertext.
fn write_file_atomic(path: &Path, contents: &[u8]) -> VaultResult<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, contents).map_err(|_| VaultError::MasterPasswordChangeFailed)?;
    fs::rename(&temporary, path).map_err(|_| VaultError::MasterPasswordChangeFailed)?;
    Ok(())
}
