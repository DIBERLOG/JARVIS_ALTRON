//! The vault session: one master-key session shared with the notes storage, and
//! its own database, key, and decrypted cache for passwords.
//!
//! ```text
//! master password -> Argon2id KEK -> unwraps the master key   (NotesVault owns this)
//!                                     |-> HKDF(JARVIS/notes/v1) -> notes database
//!                                     `-> HKDF(JARVIS/vault/v1) -> vault database
//! ```
//!
//! The password store therefore never sees the master key: it is built from a
//! [`PurposeKeyProvider`], which holds only the derived vault key. Locking drops
//! the vault key, the decrypted vault cache, and the master key together.
//!
//! Unlocking is shared with the notes feature on purpose: one master password
//! protects one encrypted storage. The *working keys* stay separate, and the
//! vault keeps its own database file, journal, and cursors.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use uuid::Uuid;

use crate::notes::vault::{NotesVault, StorageState, StorageStatus, VaultPaths};
use crate::notes::NoteError;
use crate::sync::crypto::{KeyPurpose, MasterKey, PurposeKeyProvider};
use crate::sync::sqlite::SqliteSyncRepository;
use crate::sync::DeviceId;

use super::keys::{change_master_password, MasterPasswordChange};
use super::model::*;
use super::store::{EncryptedVaultStore, VaultStore};

/// File that holds the vault database, next to the notes database.
pub const VAULT_DB_FILE: &str = "vault.sqlite3";

/// Public result type of this module.
pub type VaultResult<T> = Result<T, VaultError>;

/// Storage plus vault counts, as the interface needs them.
#[derive(Clone, Debug, Serialize)]
pub struct VaultStatus {
    /// Shared encrypted storage state: uninitialized, locked, unlocked, or
    /// missing its key.
    pub storage: StorageStatus,
    /// Vault counts; zero while locked, because nothing is decrypted.
    pub stats: VaultStats,
}

impl VaultStatus {
    pub fn is_unlocked(&self) -> bool {
        self.storage.is_unlocked()
    }
}

/// Idle timeout options offered by the interface, in seconds. Zero means never.
pub const IDLE_TIMEOUT_OPTIONS: [u64; 5] = [60, 300, 900, 1800, 0];
/// Recommended default idle timeout.
pub const DEFAULT_IDLE_TIMEOUT_SECONDS: u64 = 300;

/// Tracks how long the vault has been idle.
///
/// The policy is deliberately a plain value type so the rules (never, exact
/// timeout, activity resets the clock) can be tested without a desktop session.
#[derive(Clone, Copy, Debug)]
pub struct IdleLock {
    timeout_seconds: u64,
    last_activity: Instant,
}

impl IdleLock {
    /// Builds a lock with `timeout_seconds`; an unsupported value falls back to
    /// the default, and zero means "never lock automatically".
    pub fn new(timeout_seconds: u64) -> Self {
        Self {
            timeout_seconds: normalize_timeout(timeout_seconds),
            last_activity: Instant::now(),
        }
    }

    pub fn with_timeout_at(timeout_seconds: u64, now: Instant) -> Self {
        Self {
            timeout_seconds: normalize_timeout(timeout_seconds),
            last_activity: now,
        }
    }

    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds
    }

    pub fn is_automatic(&self) -> bool {
        self.timeout_seconds != 0
    }

    /// Records vault activity, restarting the idle clock.
    pub fn touch(&mut self, now: Instant) {
        self.last_activity = now;
    }

    /// Changes the timeout, keeping the current activity time.
    pub fn set_timeout(&mut self, timeout_seconds: u64) {
        self.timeout_seconds = normalize_timeout(timeout_seconds);
    }

    /// Whether the vault must be locked by now.
    pub fn is_due(&self, now: Instant) -> bool {
        if !self.is_automatic() {
            return false;
        }
        now.saturating_duration_since(self.last_activity)
            >= Duration::from_secs(self.timeout_seconds)
    }

    /// Whole seconds left before the automatic lock, or `None` when disabled.
    pub fn remaining_seconds(&self, now: Instant) -> Option<u64> {
        if !self.is_automatic() {
            return None;
        }
        let elapsed = now.saturating_duration_since(self.last_activity).as_secs();
        Some(self.timeout_seconds.saturating_sub(elapsed))
    }
}

/// Only the documented options are accepted; anything else becomes the default.
pub fn normalize_timeout(timeout_seconds: u64) -> u64 {
    if IDLE_TIMEOUT_OPTIONS.contains(&timeout_seconds) {
        timeout_seconds
    } else {
        DEFAULT_IDLE_TIMEOUT_SECONDS
    }
}

/// Owns the vault database and the derived vault key.
pub struct VaultSession {
    storage: NotesVault,
    database: PathBuf,
    device_id: DeviceId,
    store: Option<EncryptedVaultStore>,
}

impl VaultSession {
    /// Opens the session over `data_dir`, creating it when needed.
    pub fn open(data_dir: &Path) -> VaultResult<Self> {
        let storage = NotesVault::open(data_dir)?;
        let database = storage.paths().data_dir.join(VAULT_DB_FILE);
        let device_id = storage.device_id().clone();
        Ok(Self {
            storage,
            database,
            device_id,
            store: None,
        })
    }

    /// Opens the session in the production application data directory.
    pub fn open_production() -> VaultResult<Self> {
        Self::open(&VaultPaths::production()?.data_dir)
    }

    pub fn paths(&self) -> &VaultPaths {
        self.storage.paths()
    }

    pub fn database_path(&self) -> &Path {
        &self.database
    }

    /// Whether the shared encrypted storage is unlocked.
    pub fn is_unlocked(&self) -> bool {
        self.storage.is_unlocked()
    }

    /// Storage status plus vault counts.
    ///
    /// The vault database can already hold records while the notes database is
    /// still empty, so the reported state considers both. Reporting
    /// "uninitialized" there would invite creating a second master key and
    /// orphaning the existing ciphertext.
    pub fn status(&mut self) -> VaultResult<VaultStatus> {
        let mut storage = self.storage.status()?;
        if matches!(storage.state, StorageState::Uninitialized)
            && self.vault_database_has_records()?
        {
            storage.state = StorageState::KeyMissing;
            storage.has_stored_data = true;
        }
        let stats = if storage.is_unlocked() {
            self.store()?.stats()?
        } else {
            self.store = None;
            VaultStats::default()
        };
        Ok(VaultStatus { storage, stats })
    }

    /// Creates the master key from a master password and unlocks the session.
    ///
    /// Refused when either database already holds records: a new master key
    /// would make the existing ciphertext unreadable for good.
    pub fn initialize(&mut self, password: &str) -> VaultResult<VaultStatus> {
        if self.vault_database_has_records()? {
            return Err(VaultError::KeyMissing);
        }
        self.storage.initialize(password)?;
        self.status()
    }

    pub fn unlock_with_password(&mut self, password: &str) -> VaultResult<VaultStatus> {
        self.storage.unlock_with_password(password)?;
        self.status()
    }

    pub fn unlock_with_dpapi(&mut self) -> VaultResult<VaultStatus> {
        self.storage.unlock_with_dpapi()?;
        self.status()
    }

    pub fn import_backup(&mut self, envelope_json: &str, password: &str) -> VaultResult<VaultStatus> {
        self.storage.import_backup(envelope_json, password)?;
        self.status()
    }

    /// Re-wraps the master key under a new master password.
    pub fn change_master_password(
        &mut self,
        current_password: &str,
        new_password: &str,
    ) -> VaultResult<MasterPasswordChange> {
        change_master_password(&mut self.storage, current_password, new_password)
    }

    /// Writes a fresh portable envelope to a chosen path.
    pub fn export_backup_to(
        &mut self,
        password: &str,
        destination: &Path,
    ) -> VaultResult<PathBuf> {
        Ok(self.storage.export_backup_to(password, destination)?)
    }

    /// Drops the vault key, the decrypted cache, and the master key.
    pub fn lock(&mut self) {
        self.store = None;
        self.storage.lock();
    }

    /// Borrows the password store, building it from the unlocked master key on
    /// first use. Refuses while the storage is locked.
    pub fn store(&mut self) -> VaultResult<&mut EncryptedVaultStore> {
        if !self.storage.is_unlocked() {
            // Never keep a vault key around once the master key is gone.
            self.store = None;
            return Err(VaultError::StorageLocked);
        }
        if self.store.is_none() {
            // A copy of the master key exists only for the duration of the
            // derivation; the provider keeps the derived vault key alone.
            let master = {
                let notes = self.storage.store()?;
                MasterKey::from_bytes(*notes.crypto().key().as_array())
            };
            let provider = PurposeKeyProvider::derive(&master, KeyPurpose::Vault)?;
            let repository = SqliteSyncRepository::open(&self.database)?;
            self.store = Some(VaultStore::new(
                repository,
                provider,
                self.device_id.clone(),
            ));
        }
        Ok(self.store.as_mut().expect("just initialized"))
    }

    /// Runs `action` against the vault store, or fails when locked.
    pub fn with_store<T>(
        &mut self,
        action: impl FnOnce(&mut EncryptedVaultStore) -> VaultResult<T>,
    ) -> VaultResult<T> {
        let store = self.store()?;
        action(store)
    }

    /// Notes storage, for commands that manage the shared key lifecycle.
    pub fn storage_mut(&mut self) -> &mut NotesVault {
        &mut self.storage
    }

    /// Runs `action` against the shared key lifecycle (notes storage).
    ///
    /// The notes commands use this, so notes and the vault share exactly one
    /// unlock state while keeping separate working keys and databases.
    pub fn with_storage<T>(
        &mut self,
        action: impl FnOnce(&mut NotesVault) -> Result<T, NoteError>,
    ) -> Result<T, NoteError> {
        action(&mut self.storage)
    }

    /// Whether the vault database has any applied operation.
    pub fn has_items(&mut self) -> VaultResult<bool> {
        self.store()?.has_stored_entities()
    }

    /// Reads the vault journal without any key: used to detect an existing
    /// encrypted set before a master key is created.
    fn vault_database_has_records(&self) -> VaultResult<bool> {
        if !self.database.is_file() {
            return Ok(false);
        }
        let repository = SqliteSyncRepository::open(&self.database)?;
        Ok(!repository
            .page_after(crate::sync::SyncCursor(0), 1)?
            .operations
            .is_empty())
    }

    /// Reveals one item's password and notes. Only used by the reveal command and
    /// the Rust-side clipboard copy.
    pub fn reveal_secret(
        &mut self,
        id: Uuid,
        reveal_timeout_seconds: u64,
    ) -> VaultResult<SecretRevealResult> {
        self.with_store(|store| store.reveal_secret(id, reveal_timeout_seconds))
    }
}

impl std::fmt::Debug for VaultSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VaultSession")
            .field("database", &self.database)
            .field("unlocked", &self.is_unlocked())
            .field("vault_open", &self.store.is_some())
            .finish()
    }
}
