//! The AI-memory session: its own database and its own derived key, over the
//! shared unlock state.
//!
//! ```text
//! master password -> Argon2id KEK -> unwraps the master key    (the notes session owns this)
//!                                     |-> HKDF(JARVIS/notes/v1)      -> sync.sqlite3
//!                                     |-> HKDF(JARVIS/vault/v1)      -> vault.sqlite3
//!                                     `-> HKDF(JARVIS/ai-memory/v1)  -> ai-memory.sqlite3
//! ```
//!
//! The memory store therefore never sees the master key: it is built from a
//! [`PurposeKeyProvider`], which holds only the derived memory key. Locking drops
//! that key and the decrypted cache together with the master key, and the ciphertext
//! on disk is untouched.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::sync::crypto::PurposeKeyProvider;
use crate::sync::sqlite::SqliteSyncRepository;
use crate::sync::{DeviceId, SyncCursor};

use super::config::{linear_search_warning, MemorySettings};
use super::error::MemoryError;
use super::model::MemoryStats;
use super::store::{EncryptedMemoryStore, MemoryStore};

/// Database file that holds the encrypted AI memory, next to the other stores.
pub const AI_MEMORY_DB_FILE: &str = "ai-memory.sqlite3";

/// Where the memory database lives for a given data directory.
pub fn database_path(data_dir: &Path) -> PathBuf {
    data_dir.join(AI_MEMORY_DB_FILE)
}

/// Opens the memory store over its own database with the derived memory key.
pub fn open_store(
    database: &Path,
    provider: PurposeKeyProvider,
    device_id: DeviceId,
) -> Result<EncryptedMemoryStore, MemoryError> {
    let repository = SqliteSyncRepository::open(database)?;
    Ok(MemoryStore::new(repository, provider, device_id))
}

/// Whether the memory database holds any applied operation.
///
/// This needs no key, which is what lets the interface report "there is memory, but
/// the storage is locked" instead of pretending the database is empty.
pub fn database_has_records(database: &Path) -> Result<bool, MemoryError> {
    if !database.is_file() {
        return Ok(false);
    }
    let repository = SqliteSyncRepository::open(database)?;
    Ok(!repository
        .page_after(SyncCursor(0), 1)?
        .operations
        .is_empty())
}

/// What the interface shows about the memory layer.
#[derive(Clone, Debug, Serialize)]
pub struct MemoryStatus {
    /// The shared encrypted storage is unlocked, so memory is readable.
    pub unlocked: bool,
    /// The memory database already holds records, even while locked.
    pub has_stored_data: bool,
    pub settings: MemorySettings,
    /// Counts; zero while locked, because nothing is decrypted.
    pub stats: MemoryStats,
    /// Set when the linear scan cost has grown large enough to mention.
    pub linear_search_cost_warning: Option<String>,
}

impl MemoryStatus {
    /// Status while the storage is locked: no counts, no decryption.
    pub fn locked(settings: MemorySettings, has_stored_data: bool) -> Self {
        Self {
            unlocked: false,
            has_stored_data,
            settings,
            stats: MemoryStats::default(),
            linear_search_cost_warning: None,
        }
    }

    /// Status for an unlocked store.
    pub fn unlocked(settings: MemorySettings, stats: MemoryStats, has_stored_data: bool) -> Self {
        let warning = linear_search_warning(&stats).map(str::to_string);
        Self {
            unlocked: true,
            has_stored_data,
            settings,
            stats,
            linear_search_cost_warning: warning,
        }
    }

    /// Whether memory is switched on and usable right now.
    pub fn is_active(&self) -> bool {
        self.unlocked && self.settings.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn the_database_path_is_next_to_the_other_stores() {
        let directory = Path::new("C:/data/jarvis");
        assert_eq!(
            database_path(directory),
            directory.join("ai-memory.sqlite3")
        );
        assert_eq!(AI_MEMORY_DB_FILE, "ai-memory.sqlite3");
    }

    #[test]
    fn a_missing_database_holds_no_records() {
        let directory = tempdir().unwrap();
        let database = database_path(directory.path());
        assert!(!database_has_records(&database).unwrap());
    }

    #[test]
    fn an_empty_database_opened_with_the_memory_key_reports_no_records() {
        let directory = tempdir().unwrap();
        let database = database_path(directory.path());
        let provider = PurposeKeyProvider::derive(
            &crate::sync::crypto::random_master_key().unwrap(),
            crate::sync::crypto::KeyPurpose::AiMemory,
        )
        .unwrap();
        let store = open_store(
            &database,
            provider,
            DeviceId::new("memory_session_device").unwrap(),
        )
        .unwrap();
        assert!(!store.has_stored_entities().unwrap());
        // The file now exists, and it still holds no records.
        assert!(!database_has_records(&database).unwrap());
    }

    #[test]
    fn status_reports_locked_and_unlocked_without_leaking_counts() {
        let locked = MemoryStatus::locked(MemorySettings::default(), true);
        assert!(!locked.unlocked);
        assert!(locked.has_stored_data);
        assert!(!locked.is_active());
        assert_eq!(locked.stats.conversations, 0);
        assert!(locked.linear_search_cost_warning.is_none());

        let settings = MemorySettings::default();
        let unlocked = MemoryStatus::unlocked(
            settings,
            MemoryStats {
                conversations: 2,
                facts: 3,
                ..MemoryStats::default()
            },
            true,
        );
        assert!(unlocked.unlocked);
        assert!(unlocked.is_active());
        assert_eq!(unlocked.stats.facts, 3);
        assert!(unlocked.linear_search_cost_warning.is_none());

        // A disabled feature is not "active" even when the storage is open.
        let disabled = MemoryStatus::unlocked(
            MemorySettings {
                enabled: false,
                ..MemorySettings::default()
            },
            MemoryStats::default(),
            true,
        );
        assert!(!disabled.is_active());

        // A large memory reports the linear cost warning.
        let large = MemoryStatus::unlocked(
            MemorySettings::default(),
            MemoryStats {
                facts: super::super::config::LINEAR_SEARCH_WARNING_FACTS + 1,
                ..MemoryStats::default()
            },
            true,
        );
        assert_eq!(
            large.linear_search_cost_warning.as_deref(),
            Some("memory_search_cost_facts")
        );
    }
}
