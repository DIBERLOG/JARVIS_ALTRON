//! The vault session: one master-key session shared with the notes storage and the
//! encrypted AI memory, and its own database, key, and decrypted cache for
//! passwords.
//!
//! ```text
//! master password -> Argon2id KEK -> unwraps the master key   (NotesVault owns this)
//!                                     |-> HKDF(JARVIS/notes/v1)      -> sync.sqlite3
//!                                     |-> HKDF(JARVIS/vault/v1)      -> vault.sqlite3
//!                                     `-> HKDF(JARVIS/ai-memory/v1)  -> ai-memory.sqlite3
//! ```
//!
//! Each feature therefore never sees the master key: it is built from a
//! [`PurposeKeyProvider`], which holds only its own derived key. Locking drops the
//! derived keys, the decrypted caches, and the master key together.
//!
//! Unlocking is shared on purpose: one master password protects one encrypted
//! storage. The *working keys*, the databases, the journals, and the cursors stay
//! separate, and each feature keeps its own error type.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use uuid::Uuid;

use crate::memory::store::EncryptedMemoryStore;
use crate::memory::{MemoryError, MemoryStatus};
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
    memory_database: PathBuf,
    device_id: DeviceId,
    store: Option<EncryptedVaultStore>,
    /// The AI-memory store, built from its own derived key on first use.
    memory: Option<EncryptedMemoryStore>,
}

impl VaultSession {
    /// Opens the session over `data_dir`, creating it when needed.
    pub fn open(data_dir: &Path) -> VaultResult<Self> {
        let storage = NotesVault::open(data_dir)?;
        let database = storage.paths().data_dir.join(VAULT_DB_FILE);
        let memory_database = crate::memory::database_path(&storage.paths().data_dir);
        let device_id = storage.device_id().clone();
        Ok(Self {
            storage,
            database,
            memory_database,
            device_id,
            store: None,
            memory: None,
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

    /// Path of the encrypted AI-memory database.
    pub fn memory_database_path(&self) -> &Path {
        &self.memory_database
    }

    /// Stable device identifier shared by every store in this session.
    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
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

    /// The shared storage state on its own, without touching the vault store.
    ///
    /// The AI-memory commands use this: they need the unlock gate, not the password
    /// counts, and opening the vault store for them would be wasted work.
    pub fn storage_status(&mut self) -> VaultResult<StorageStatus> {
        let mut storage = self.storage.status()?;
        if matches!(storage.state, StorageState::Uninitialized)
            && (self.vault_database_has_records()? || self.has_memory_records())
        {
            storage.state = StorageState::KeyMissing;
            storage.has_stored_data = true;
        }
        Ok(storage)
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

    pub fn import_backup(
        &mut self,
        envelope_json: &str,
        password: &str,
    ) -> VaultResult<VaultStatus> {
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
    pub fn export_backup_to(&mut self, password: &str, destination: &Path) -> VaultResult<PathBuf> {
        Ok(self.storage.export_backup_to(password, destination)?)
    }

    /// Drops the vault key, the memory key, the decrypted caches, and the master
    /// key.
    ///
    /// The memory store is dropped with the vault store on purpose: every derived
    /// key must disappear together with the master key it came from, whatever the
    /// lock path was (idle timeout, explicit lock, or application exit).
    pub fn lock(&mut self) {
        self.store = None;
        self.memory = None;
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

    // ------------------------------------------------------------ ai memory

    /// Borrows the AI-memory store, building it from the unlocked master key on
    /// first use. Refuses while the shared storage is locked.
    ///
    /// The memory key is derived from the master key with `JARVIS/ai-memory/v1`, so
    /// it is a different key than the notes and vault keys, and it lives in a
    /// different database file.
    pub fn memory_store(&mut self) -> Result<&mut EncryptedMemoryStore, MemoryError> {
        if !self.storage.is_unlocked() {
            // Never keep a memory key around once the master key is gone.
            self.memory = None;
            return Err(MemoryError::StorageLocked);
        }
        if self.memory.is_none() {
            let master = {
                let notes = self
                    .storage
                    .store()
                    .map_err(|_| MemoryError::StorageLocked)?;
                MasterKey::from_bytes(*notes.crypto().key().as_array())
            };
            let provider = PurposeKeyProvider::derive(&master, KeyPurpose::AiMemory)
                .map_err(|_| MemoryError::InvalidConfiguration)?;
            let store = crate::memory::open_memory_store(
                &self.memory_database,
                provider,
                self.device_id.clone(),
            )?;
            self.memory = Some(store);
        }
        self.memory.as_mut().ok_or(MemoryError::StorageLocked)
    }

    /// Runs `action` against the AI-memory store, or fails when locked.
    pub fn with_memory<T>(
        &mut self,
        action: impl FnOnce(&mut EncryptedMemoryStore) -> Result<T, MemoryError>,
    ) -> Result<T, MemoryError> {
        let store = self.memory_store()?;
        action(store)
    }

    /// Whether the AI-memory database holds records. Needs no key.
    pub fn has_memory_records(&self) -> bool {
        crate::memory::database_has_records(&self.memory_database).unwrap_or(false)
    }

    /// Status of the memory layer for the interface.
    ///
    /// While locked nothing is decrypted, and the counts stay zero: the interface
    /// shows the storage gate instead of pretending the memory is empty.
    pub fn memory_status(
        &mut self,
        settings: crate::memory::MemorySettings,
    ) -> Result<MemoryStatus, MemoryError> {
        let has_stored_data = self.has_memory_records();
        if !self.storage.is_unlocked() {
            self.memory = None;
            return Ok(MemoryStatus::locked(settings, has_stored_data));
        }
        if !settings.enabled {
            // Memory is switched off: report the switch without decrypting anything
            // the user asked not to use.
            self.memory = None;
            return Ok(MemoryStatus {
                unlocked: true,
                has_stored_data,
                settings,
                stats: crate::memory::MemoryStats::default(),
                linear_search_cost_warning: None,
            });
        }
        let stats = self.memory_store()?.stats()?;
        Ok(MemoryStatus::unlocked(settings, stats, has_stored_data))
    }

    /// Drops the derived memory key and the decrypted memory cache.
    ///
    /// Used when the user switches memory off: the shared unlock state is kept, so the
    /// notes and the password vault stay usable, while no memory key is held for a
    /// feature the user asked not to use.
    pub fn drop_memory(&mut self) {
        self.memory = None;
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
            .field("memory_database", &self.memory_database)
            .field("unlocked", &self.is_unlocked())
            .field("vault_open", &self.store.is_some())
            .field("memory_open", &self.memory.is_some())
            .finish()
    }
}
