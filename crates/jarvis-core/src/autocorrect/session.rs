//! The autocorrect session: its own database and its own derived key, over the shared
//! unlock state.
//!
//! ```text
//! master password -> Argon2id KEK -> unwraps the master key    (the notes session owns this)
//!                                     |-> HKDF(JARVIS/notes/v1)        -> sync.sqlite3
//!                                     |-> HKDF(JARVIS/vault/v1)        -> vault.sqlite3
//!                                     |-> HKDF(JARVIS/ai-memory/v1)    -> ai-memory.sqlite3
//!                                     `-> HKDF(JARVIS/autocorrect/v1)  -> autocorrect.sqlite3
//! ```
//!
//! The word list therefore never sees the master key: it is built from a
//! [`PurposeKeyProvider`], which holds only the derived dictionary key. Locking drops that
//! key and the decrypted words together with the master key, and the ciphertext on disk is
//! untouched.
//!
//! The Hunspell dictionaries live next to the database in a plain folder, because they are
//! public files the user installs (`docs/AUTOCORRECT.md` records where to get them and
//! what they must hash to). Only the user's own words are encrypted, because only those
//! say something about the user.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::sync::crypto::PurposeKeyProvider;
use crate::sync::sqlite::SqliteSyncRepository;
use crate::sync::{DeviceId, SyncCursor};

use super::dictionary::{DictionaryManager, DictionaryState, DICTIONARIES_DIR};
use super::error::AutocorrectError;
use super::model::Language;
use super::settings::AutocorrectSettings;
use super::user_dictionary::{EncryptedUserDictionary, UserDictionaryStats, UserDictionaryStore};

/// Database file that holds the encrypted user dictionary, next to the other stores.
pub const AUTOCORRECT_DB_FILE: &str = "autocorrect.sqlite3";

/// Where the autocorrect database lives for a given data directory.
pub fn database_path(data_dir: &Path) -> PathBuf {
    data_dir.join(AUTOCORRECT_DB_FILE)
}

/// Where the Hunspell dictionaries are read from for a given data directory.
pub fn dictionary_directory(data_dir: &Path) -> PathBuf {
    data_dir.join(DICTIONARIES_DIR)
}

/// The folder the settings ask for, or the application data folder.
pub fn resolved_dictionary_directory(data_dir: &Path, settings: &AutocorrectSettings) -> PathBuf {
    match settings.dictionary_dir.as_deref() {
        Some(custom) if !custom.trim().is_empty() => PathBuf::from(custom.trim()),
        _ => dictionary_directory(data_dir),
    }
}

/// Opens the user dictionary over its own database with the derived dictionary key.
pub fn open_user_dictionary(
    database: &Path,
    provider: PurposeKeyProvider,
    device_id: DeviceId,
) -> Result<EncryptedUserDictionary, AutocorrectError> {
    let repository = SqliteSyncRepository::open(database)?;
    Ok(UserDictionaryStore::new(repository, provider, device_id))
}

/// Whether the autocorrect database holds any applied operation.
///
/// This needs no key, which is what lets the interface report "you have saved words, but
/// the storage is locked" instead of pretending the list is empty.
pub fn database_has_records(database: &Path) -> Result<bool, AutocorrectError> {
    if !database.is_file() {
        return Ok(false);
    }
    let repository = SqliteSyncRepository::open(database)?;
    Ok(!repository
        .page_after(SyncCursor(0), 1)?
        .operations
        .is_empty())
}

/// What the interface shows about the autocorrect layer.
#[derive(Clone, Debug, Serialize)]
pub struct AutocorrectStatus {
    /// The shared encrypted storage is unlocked, so the word list is readable.
    pub unlocked: bool,
    /// The word database already holds entries, even while locked.
    pub has_stored_words: bool,
    pub settings: AutocorrectSettings,
    /// One entry per supported language: ready, missing, or invalid.
    pub dictionaries: Vec<DictionaryState>,
    /// Folder the dictionaries are read from.
    pub dictionary_dir: String,
    /// Words whose language has no usable dictionary.
    pub unavailable: Vec<Language>,
    pub user: UserDictionaryStats,
    /// Whether the AI improvement action is offered at all.
    pub ai_improvement_available: bool,
}

impl AutocorrectStatus {
    /// Builds the status for a locked storage: nothing is decrypted.
    pub fn locked(
        settings: AutocorrectSettings,
        has_stored_words: bool,
        dictionaries: Vec<DictionaryState>,
        dictionary_dir: PathBuf,
        ai_improvement_available: bool,
    ) -> Self {
        let unavailable = unavailable_languages(&dictionaries);
        Self {
            unlocked: false,
            has_stored_words,
            settings,
            dictionaries,
            dictionary_dir: dictionary_dir.display().to_string(),
            unavailable,
            user: UserDictionaryStats::default(),
            ai_improvement_available,
        }
    }

    /// Whether spelling checks can run right now.
    pub fn is_checkable(&self) -> bool {
        self.settings.enabled && self.unavailable.len() < Language::all().len()
    }

    /// Whether the user's own word list can be read right now.
    pub fn has_user_dictionary(&self) -> bool {
        self.unlocked
    }
}

/// Languages whose dictionary is not usable, from a status list.
pub fn unavailable_languages(dictionaries: &[DictionaryState]) -> Vec<Language> {
    Language::all()
        .iter()
        .copied()
        .filter(|language| {
            !dictionaries
                .iter()
                .any(|state| state.language() == *language && state.is_ready())
        })
        .collect()
}

/// Builds the dictionary states for a folder.
pub fn dictionary_states(manager: &DictionaryManager) -> Vec<DictionaryState> {
    manager.states()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::crypto::{random_master_key, KeyPurpose, PurposeKeyProvider};
    use tempfile::tempdir;

    fn provider() -> PurposeKeyProvider {
        PurposeKeyProvider::derive(&random_master_key().unwrap(), KeyPurpose::Autocorrect).unwrap()
    }

    #[test]
    fn the_paths_are_next_to_the_other_stores() {
        let directory = Path::new("C:/data/jarvis");
        assert_eq!(
            database_path(directory),
            directory.join("autocorrect.sqlite3")
        );
        assert_eq!(AUTOCORRECT_DB_FILE, "autocorrect.sqlite3");
        assert_eq!(
            dictionary_directory(directory),
            directory.join("dictionaries")
        );
    }

    #[test]
    fn a_custom_dictionary_folder_wins_over_the_default() {
        let directory = Path::new("C:/data/jarvis");
        let default = AutocorrectSettings::default();
        assert_eq!(
            resolved_dictionary_directory(directory, &default),
            directory.join("dictionaries")
        );
        let custom = AutocorrectSettings {
            dictionary_dir: Some("D:/dicts".to_string()),
            ..AutocorrectSettings::default()
        };
        assert_eq!(
            resolved_dictionary_directory(directory, &custom),
            PathBuf::from("D:/dicts")
        );
        // A blank setting falls back instead of pointing at the drive root.
        let blank = AutocorrectSettings {
            dictionary_dir: Some("   ".to_string()),
            ..AutocorrectSettings::default()
        };
        assert_eq!(
            resolved_dictionary_directory(directory, &blank),
            directory.join("dictionaries")
        );
    }

    #[test]
    fn a_missing_database_holds_no_records() {
        let directory = tempdir().unwrap();
        let database = database_path(directory.path());
        assert!(!database_has_records(&database).unwrap());
    }

    #[test]
    fn an_empty_database_opened_with_the_dictionary_key_reports_no_records() {
        let directory = tempdir().unwrap();
        let database = database_path(directory.path());
        let store = open_user_dictionary(
            &database,
            provider(),
            DeviceId::new("autocorrect_session_device").unwrap(),
        )
        .unwrap();
        assert!(!store.has_stored_entities().unwrap());
        assert!(!database_has_records(&database).unwrap());
    }

    #[test]
    fn another_purpose_key_cannot_read_a_dictionary_database() {
        let directory = tempdir().unwrap();
        let database = database_path(directory.path());
        let mut store = open_user_dictionary(
            &database,
            provider(),
            DeviceId::new("autocorrect_session_device").unwrap(),
        )
        .unwrap();
        store.add("Проверка", Language::Russian, false).unwrap();
        assert!(database_has_records(&database).unwrap());

        // The same database opened with a different derived key holds entries that
        // cannot be decrypted: they are reported as unreadable, not as words.
        let other =
            PurposeKeyProvider::derive(&random_master_key().unwrap(), KeyPurpose::Autocorrect)
                .unwrap();
        let mut foreign = open_user_dictionary(
            &database,
            other,
            DeviceId::new("autocorrect_session_device").unwrap(),
        )
        .unwrap();
        assert_eq!(foreign.stats().unwrap().words, 0);
        assert_eq!(foreign.stats().unwrap().unreadable, 1);
    }

    #[test]
    fn a_locked_status_decrypts_nothing_and_names_the_missing_dictionaries() {
        let states = vec![
            DictionaryState::Missing {
                language: Language::Russian,
                expected: vec!["ru_RU.aff".to_string()],
            },
            DictionaryState::Missing {
                language: Language::English,
                expected: vec!["en_US.aff".to_string()],
            },
        ];
        let status = AutocorrectStatus::locked(
            AutocorrectSettings::default(),
            true,
            states,
            PathBuf::from("C:/data/jarvis/dictionaries"),
            false,
        );
        assert!(!status.unlocked);
        assert!(status.has_stored_words);
        assert!(!status.is_checkable());
        assert!(!status.has_user_dictionary());
        assert_eq!(status.user.words, 0);
        assert_eq!(status.unavailable.len(), 2);
        assert_eq!(status.dictionary_dir, "C:/data/jarvis/dictionaries");
        assert!(!status.ai_improvement_available);
    }

    #[test]
    fn one_ready_dictionary_is_enough_to_check_that_language() {
        let states = vec![
            DictionaryState::Ready {
                language: Language::Russian,
                aff: "ru_RU.aff".to_string(),
                dic: "ru_RU.dic".to_string(),
                words: Some(10),
                source: None,
            },
            DictionaryState::Missing {
                language: Language::English,
                expected: vec!["en_US.aff".to_string()],
            },
        ];
        assert_eq!(unavailable_languages(&states), vec![Language::English]);
        let status = AutocorrectStatus::locked(
            AutocorrectSettings::default(),
            false,
            states,
            PathBuf::from("dicts"),
            true,
        );
        assert!(status.is_checkable());
        assert!(status.ai_improvement_available);
    }
}
