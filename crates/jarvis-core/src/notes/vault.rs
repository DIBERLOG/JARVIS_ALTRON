//! Master-key lifecycle for encrypted local storage.
//!
//! Four files live in the application data directory:
//!
//! ```text
//! sync.sqlite3      entity revisions, cursors, and encrypted payloads
//! key.dpapi         DPAPI-protected copy of the random master key (this user)
//! key.backup.json   portable, master-password protected envelope for the key
//! device.id         stable device identifier (technical metadata, not secret)
//! ```
//!
//! The master password is never stored. It is verified by unwrapping
//! `key.backup.json`, which also yields the master key. DPAPI unlock is the
//! convenience path for the current Windows user; the portable envelope is the
//! only path that works on another machine.
//!
//! While locked, no note content is read or produced: the repository is held
//! without any key material at all.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use uuid::Uuid;

use crate::sync::crypto::{
    dpapi_protect, dpapi_unprotect, export_backup, import_backup, random_master_key,
    DpapiProtectedKey, MasterKeyCryptoProvider, PortableKeyBackup,
};
use crate::sync::sqlite::SqliteSyncRepository;
use crate::sync::{DeviceId, SyncCursor};

use super::model::*;
use super::store::{EncryptedNoteStore, NoteStore};

/// File holding the DPAPI-protected master key.
pub const DPAPI_KEY_FILE: &str = "key.dpapi";
/// File holding the portable, password-protected master-key envelope.
pub const BACKUP_KEY_FILE: &str = "key.backup.json";
/// File holding the stable device identifier.
pub const DEVICE_ID_FILE: &str = "device.id";

/// Whether the storage is usable right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageState {
    /// No key material and no stored entities: a master password must be created.
    Uninitialized,
    /// Key material exists but the master key has not been supplied yet.
    Locked,
    /// The master key is in memory; note content is available.
    Unlocked,
    /// Entities exist but no key file remains: only a backup import can recover.
    KeyMissing,
}

/// Everything the interface needs to render the storage gate.
#[derive(Clone, Debug, Serialize)]
pub struct StorageStatus {
    pub state: StorageState,
    /// A DPAPI-protected key exists for this Windows user.
    pub dpapi_available: bool,
    /// A portable backup envelope exists on this machine.
    pub backup_available: bool,
    /// Storage already holds applied operations.
    pub has_stored_data: bool,
    /// Directory that holds the database and key files.
    pub data_dir: String,
    /// Note counts; zero while locked, because nothing is decrypted.
    pub stats: NoteStats,
}

impl StorageStatus {
    pub fn is_unlocked(&self) -> bool {
        matches!(self.state, StorageState::Unlocked)
    }
}

/// Paths used by the vault.
#[derive(Clone, Debug)]
pub struct VaultPaths {
    pub data_dir: PathBuf,
    pub database: PathBuf,
    pub dpapi_key: PathBuf,
    pub backup_key: PathBuf,
    pub device_id: PathBuf,
}

impl VaultPaths {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
            database: data_dir.join("sync.sqlite3"),
            dpapi_key: data_dir.join(DPAPI_KEY_FILE),
            backup_key: data_dir.join(BACKUP_KEY_FILE),
            device_id: data_dir.join(DEVICE_ID_FILE),
        }
    }

    /// Production paths, derived from the per-user application data directory.
    pub fn production() -> Result<Self, NoteError> {
        let database = SqliteSyncRepository::production_path()?;
        let data_dir = database
            .parent()
            .map(Path::to_path_buf)
            .ok_or(NoteError::VaultIo)?;
        Ok(Self::new(&data_dir))
    }
}

/// Owns the encrypted storage and its key-material lifecycle.
///
/// Exactly one of `repository` (locked) and `store` (unlocked) is present.
pub struct NotesVault {
    paths: VaultPaths,
    repository: Option<SqliteSyncRepository>,
    store: Option<EncryptedNoteStore>,
    device_id: DeviceId,
}

impl NotesVault {
    /// Opens the vault at `data_dir`, creating the directory when needed.
    pub fn open(data_dir: &Path) -> Result<Self, NoteError> {
        let paths = VaultPaths::new(data_dir);
        fs::create_dir_all(&paths.data_dir).map_err(|_| NoteError::VaultIo)?;
        let repository = SqliteSyncRepository::open(&paths.database)?;
        let device_id = load_or_create_device_id(&paths.device_id)?;
        Ok(Self {
            paths,
            repository: Some(repository),
            store: None,
            device_id,
        })
    }

    /// Opens the vault in the production application data directory.
    pub fn open_production() -> Result<Self, NoteError> {
        Self::open(&VaultPaths::production()?.data_dir)
    }

    pub fn paths(&self) -> &VaultPaths {
        &self.paths
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    pub fn is_unlocked(&self) -> bool {
        self.store.is_some()
    }

    /// Current state, without decrypting anything while locked.
    pub fn status(&mut self) -> Result<StorageStatus, NoteError> {
        let has_stored_data = !self
            .repository()?
            .page_after(SyncCursor(0), 1)?
            .operations
            .is_empty();
        let dpapi_available = self.paths.dpapi_key.is_file();
        let backup_available = self.paths.backup_key.is_file();
        let stats = match self.store.as_mut() {
            Some(store) => store.stats()?,
            None => NoteStats::default(),
        };
        let state = if self.is_unlocked() {
            StorageState::Unlocked
        } else if dpapi_available || backup_available {
            StorageState::Locked
        } else if has_stored_data {
            StorageState::KeyMissing
        } else {
            StorageState::Uninitialized
        };
        Ok(StorageStatus {
            state,
            dpapi_available,
            backup_available,
            has_stored_data,
            data_dir: self.paths.data_dir.display().to_string(),
            stats,
        })
    }

    /// Creates the master key from a master password and unlocks the storage.
    ///
    /// Refused once key material or stored entities exist, so an existing
    /// encrypted set can never be orphaned by a second initialization.
    pub fn initialize(&mut self, password: &str) -> Result<StorageStatus, NoteError> {
        if self.paths.dpapi_key.is_file() || self.paths.backup_key.is_file() {
            return Err(NoteError::AlreadyInitialized);
        }
        if self.has_stored_entities()? {
            return Err(NoteError::KeyMissing);
        }

        let key = random_master_key()?;
        // The portable envelope is the primary artifact: it is what makes the
        // data recoverable on another machine.
        let envelope = export_backup(&key, password.as_bytes())?;
        write_file_atomic(&self.paths.backup_key, envelope.to_json()?.as_bytes())?;
        // DPAPI is a convenience for this machine and is skipped where the
        // platform cannot provide it; the password path still works.
        if let Ok(blob) = dpapi_protect(&key) {
            write_file_atomic(&self.paths.dpapi_key, blob.as_bytes())?;
        }
        self.unlock_with_key(key)?;
        self.status()
    }

    /// Unlocks with the DPAPI-protected key of the current Windows user.
    pub fn unlock_with_dpapi(&mut self) -> Result<StorageStatus, NoteError> {
        let bytes = fs::read(&self.paths.dpapi_key).map_err(|_| NoteError::VaultIo)?;
        let key = dpapi_unprotect(&DpapiProtectedKey::from_bytes(bytes))
            .map_err(NoteError::Crypto)?;
        self.unlock_with_key(key)?;
        self.status()
    }

    /// Unlocks by unwrapping the locally stored portable envelope.
    pub fn unlock_with_password(&mut self, password: &str) -> Result<StorageStatus, NoteError> {
        let envelope = self.read_local_backup()?;
        let key = import_backup(&envelope, password.as_bytes()).map_err(NoteError::Crypto)?;
        self.unlock_with_key(key)?;
        self.status()
    }

    /// Unlocks from an external portable backup and re-creates the local copies.
    ///
    /// This is the recovery path for a new machine or a reinstalled system.
    pub fn import_backup(
        &mut self,
        envelope_json: &str,
        password: &str,
    ) -> Result<StorageStatus, NoteError> {
        let envelope =
            PortableKeyBackup::from_json(envelope_json).map_err(NoteError::Crypto)?;
        let key = import_backup(&envelope, password.as_bytes()).map_err(NoteError::Crypto)?;
        write_file_atomic(&self.paths.backup_key, envelope_json.as_bytes())?;
        if let Ok(blob) = dpapi_protect(&key) {
            write_file_atomic(&self.paths.dpapi_key, blob.as_bytes())?;
        }
        self.unlock_with_key(key)?;
        self.status()
    }

    /// Builds a fresh portable envelope for the in-memory master key.
    ///
    /// Requires an unlocked storage, because the master key must be in memory;
    /// the master password itself is never stored or returned.
    pub fn export_backup_json(&mut self, password: &str) -> Result<String, NoteError> {
        let store = self.store.as_ref().ok_or(NoteError::StorageLocked)?;
        let key = store
            .crypto()
            .key();
        let envelope = export_backup(key, password.as_bytes()).map_err(NoteError::Crypto)?;
        envelope.to_json().map_err(NoteError::Crypto)
    }

    /// Writes a portable envelope to `destination`.
    pub fn export_backup_to(
        &mut self,
        password: &str,
        destination: &Path,
    ) -> Result<PathBuf, NoteError> {
        let json = self.export_backup_json(password)?;
        write_file_atomic(destination, json.as_bytes())?;
        Ok(destination.to_path_buf())
    }

    /// Copies the locally stored portable envelope to `destination`.
    ///
    /// Needs no key material: the file is already password-protected.
    pub fn copy_local_backup_to(&self, destination: &Path) -> Result<PathBuf, NoteError> {
        let contents = fs::read(&self.paths.backup_key).map_err(|_| NoteError::VaultIo)?;
        write_file_atomic(destination, &contents)?;
        Ok(destination.to_path_buf())
    }

    /// Drops the master key and returns to the locked state.
    pub fn lock(&mut self) {
        if let Some(store) = self.store.take() {
            // Dropping the store drops the master key, which zeroizes itself.
            self.repository = Some(store.into_repository());
        }
    }

    /// Borrows the note store, refusing while the storage is locked.
    pub fn store(&mut self) -> Result<&mut EncryptedNoteStore, NoteError> {
        self.store.as_mut().ok_or(NoteError::StorageLocked)
    }

    /// Runs `action` against the note store, or fails when locked.
    pub fn with_store<T>(
        &mut self,
        action: impl FnOnce(&mut EncryptedNoteStore) -> Result<T, NoteError>,
    ) -> Result<T, NoteError> {
        let store = self.store()?;
        action(store)
    }

    fn repository(&self) -> Result<&SqliteSyncRepository, NoteError> {
        self.repository
            .as_ref()
            .or_else(|| self.store.as_ref().map(|store| store.repository()))
            .ok_or(NoteError::VaultIo)
    }

    fn has_stored_entities(&self) -> Result<bool, NoteError> {
        Ok(!self
            .repository()?
            .page_after(SyncCursor(0), 1)?
            .operations
            .is_empty())
    }

    fn read_local_backup(&self) -> Result<PortableKeyBackup, NoteError> {
        let contents =
            fs::read_to_string(&self.paths.backup_key).map_err(|_| NoteError::VaultIo)?;
        PortableKeyBackup::from_json(&contents).map_err(NoteError::Crypto)
    }

    fn unlock_with_key(&mut self, key: crate::sync::crypto::MasterKey) -> Result<(), NoteError> {
        let repository = match self.repository.take() {
            Some(repository) => repository,
            None => self
                .store
                .take()
                .map(|store| store.into_repository())
                .ok_or(NoteError::VaultIo)?,
        };
        let store = NoteStore::new(
            repository,
            MasterKeyCryptoProvider::new(key),
            self.device_id.clone(),
        );
        self.store = Some(store);
        Ok(())
    }
}

impl std::fmt::Debug for NotesVault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NotesVault")
            .field("data_dir", &self.paths.data_dir)
            .field("unlocked", &self.is_unlocked())
            .field("device_id", &self.device_id)
            .finish()
    }
}

/// Loads the stable device identifier, creating it when absent.
fn load_or_create_device_id(path: &Path) -> Result<DeviceId, NoteError> {
    if let Ok(contents) = fs::read_to_string(path) {
        if let Ok(device_id) = DeviceId::new(contents.trim().to_string()) {
            return Ok(device_id);
        }
    }
    let generated = format!("windows-{}", Uuid::new_v4().simple());
    let device_id = DeviceId::new(generated).map_err(|_| NoteError::VaultIo)?;
    write_file_atomic(path, device_id.as_str().as_bytes())?;
    Ok(device_id)
}

/// Writes through a temporary file so a crash cannot leave a truncated key.
///
/// The temporary file only ever holds ciphertext (a DPAPI blob or a portable
/// envelope) or a non-secret device identifier, never note content.
fn write_file_atomic(path: &Path, contents: &[u8]) -> Result<(), NoteError> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, contents).map_err(|_| NoteError::VaultIo)?;
    fs::rename(&temporary, path).map_err(|_| NoteError::VaultIo)?;
    Ok(())
}
