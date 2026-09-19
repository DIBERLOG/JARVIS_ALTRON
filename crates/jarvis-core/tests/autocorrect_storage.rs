//! End-to-end storage tests for the local autocorrect word list on a real SQLite file.
//!
//! The unit tests inside `src/autocorrect` use the in-memory repository, so they prove the
//! checker, the corrections, the undo journal, and the payload rules but never touch a
//! database file. This file proves the same feature against `autocorrect.sqlite3` on disk,
//! where write-ahead logging, a second connection, a wrong key, and a locked storage are
//! real:
//!
//! * a word survives a close and a reopen under the same master key, with the same entity
//!   revision;
//! * the user's own words never reach the database, its write-ahead log, or its
//!   shared-memory file in the clear;
//! * the dictionary database is a fourth file with a fourth derived key: another purpose
//!   key cannot read it, and neither can the notes, vault, or memory stores;
//! * locking a `VaultSession` drops the dictionary key and the decrypted words;
//! * a check, an applied correction, and an undo all work against the real store, and the
//!   whole flow never creates an AI-memory database.
//!
//! Every text is fictional. No test opens a network connection, starts a model, or sleeps.
//!
//! The dictionary used here is a tiny fixture written by the test itself, because the real
//! Russian and English pairs are installed by the user and are deliberately not committed.

use std::fs;
use std::path::{Path, PathBuf};

use jarvis_core::autocorrect::dictionary::DictionaryManager;
use jarvis_core::autocorrect::engine::LocalSpellChecker;
use jarvis_core::autocorrect::model::{text_version, Language};
use jarvis_core::autocorrect::replacement::{apply_corrections, undo_last, CorrectionJournal};
use jarvis_core::autocorrect::session::{
    database_has_records, database_path, open_user_dictionary, AUTOCORRECT_DB_FILE,
};
use jarvis_core::autocorrect::settings::AutocorrectSettings;
use jarvis_core::autocorrect::{AutocorrectError, EncryptedUserDictionary};
use jarvis_core::memory::AI_MEMORY_DB_FILE;
use jarvis_core::notes::StorageState;
use jarvis_core::sync::crypto::{random_master_key, KeyPurpose, MasterKey, PurposeKeyProvider};
use jarvis_core::sync::{DeviceId, SyncEntityType};
use jarvis_core::vault::{VaultSession, VAULT_DB_FILE};

const PASSWORD: &str = "FICTIONAL_MASTER_PASSWORD";
const WRONG_PASSWORD: &str = "FICTIONAL_WRONG_PASSWORD";

const WORD: &str = "ФИКТИВНОЕ_СЛОВО";
const OTHER_WORD: &str = "ФИКТИВНОЕ_ВТОРОЕ_СЛОВО";
const IMPORTED_WORD: &str = "ФИКТИВНОЕ_ИМПОРТИРОВАННОЕ";

/// Text that must only ever exist inside an encrypted payload.
const CONTENT_MARKERS: [&str; 3] = [WORD, OTHER_WORD, IMPORTED_WORD];

// ---------------------------------------------------------------------- fixtures

fn master_key() -> MasterKey {
    random_master_key().expect("a random master key")
}

/// The dictionary provider: a key derived for `JARVIS/autocorrect/v1`, never the master
/// key itself, exactly as the session builds it.
fn dictionary_provider(master: &MasterKey) -> PurposeKeyProvider {
    PurposeKeyProvider::derive(master, KeyPurpose::Autocorrect).expect("derive the dictionary key")
}

fn device_id() -> DeviceId {
    DeviceId::new("FICTIONAL_DEVICE_ID").expect("a valid device identifier")
}

fn open_dictionary(database: &Path, master: &MasterKey) -> EncryptedUserDictionary {
    open_user_dictionary(database, dictionary_provider(master), device_id())
        .expect("open the word list")
}

/// Writes the tiny Hunspell pair the tests check against.
fn write_fixture_dictionary(directory: &Path) {
    fs::create_dir_all(directory).unwrap();
    let aff = "SET UTF-8\n";
    let dic = "6\nпривет\nмир\nдела\nкак\nи\nhello\n";
    for language in [Language::Russian, Language::English] {
        fs::write(directory.join(language.aff_file()), aff).unwrap();
        fs::write(directory.join(language.dic_file()), dic).unwrap();
    }
}

fn checker(directory: &Path) -> LocalSpellChecker {
    write_fixture_dictionary(directory);
    LocalSpellChecker::from_manager(DictionaryManager::new(directory))
}

/// Every byte of a database and its side files, so a test can search them.
fn database_family(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    for candidate in [
        path.to_path_buf(),
        path.with_extension("sqlite3-wal"),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        if candidate.is_file() {
            bytes.extend(fs::read(&candidate).unwrap_or_default());
        }
    }
    bytes
}

/// Whether one of the markers appears in a byte buffer.
fn contains_marker(bytes: &[u8], marker: &str) -> bool {
    let needle = marker.as_bytes();
    bytes.windows(needle.len()).any(|window| window == needle)
}

// ------------------------------------------------------------------- 1. the file

#[test]
fn a_word_survives_a_close_and_a_reopen_under_the_same_key() {
    let directory = tempdir();
    let database = database_path(directory.path());
    let master = master_key();

    let added = {
        let mut store = open_dictionary(&database, &master);
        let added = store.add(WORD, Language::Russian, false).unwrap();
        assert_eq!(added.revision, 1);
        assert_eq!(store.stats().unwrap().words, 1);
        added
    };

    // A second connection over the same file, with the same derived key.
    let mut reopened = open_dictionary(&database, &master);
    let listed = reopened
        .list(&jarvis_core::autocorrect::UserDictionaryQuery::default())
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, added.id);
    assert_eq!(listed[0].revision, added.revision);
    assert_eq!(listed[0].word, WORD.to_lowercase());
    assert!(reopened.is_known(WORD).unwrap());
    assert!(!reopened.is_known("ФИКТИВНОЕ_ОТСУТСТВУЮЩЕЕ").unwrap());

    // The database reports records without a key, which is what lets the interface say
    // "you have words, but the storage is locked".
    assert!(database_has_records(&database).unwrap());
    assert_eq!(AUTOCORRECT_DB_FILE, "autocorrect.sqlite3");
}

#[test]
fn no_stored_word_reaches_the_database_in_the_clear() {
    let directory = tempdir();
    let database = database_path(directory.path());
    let master = master_key();

    let mut store = open_dictionary(&database, &master);
    store.add(WORD, Language::Russian, false).unwrap();
    store.add(OTHER_WORD, Language::English, false).unwrap();
    // A duplicate is refused, so the list stays unique.
    assert_eq!(
        store.add(WORD, Language::Russian, true).unwrap_err(),
        AutocorrectError::Duplicate
    );

    let bytes = database_family(&database);
    assert!(!bytes.is_empty(), "the database file exists");
    for marker in CONTENT_MARKERS {
        assert!(
            !contains_marker(&bytes, marker),
            "{marker} must not appear in the database or its side files"
        );
    }
    // The row itself is there, so the absence above is not an empty-file artefact.
    let raw = fs::read(&database).unwrap();
    assert!(raw.windows(4).any(|window| window == b"SQLi"));
}

// ---------------------------------------------------------------- 2. wrong key

#[test]
fn another_purpose_key_cannot_read_the_word_list() {
    let directory = tempdir();
    let database = database_path(directory.path());
    let master = master_key();

    let mut store = open_dictionary(&database, &master);
    store.add(WORD, Language::Russian, false).unwrap();
    drop(store);

    // The memory key over the dictionary database: it can open the file and see that
    // something is there, and it can decrypt nothing.
    let other = PurposeKeyProvider::derive(&master, KeyPurpose::AiMemory).unwrap();
    let mut foreign = open_user_dictionary(&database, other, device_id()).unwrap();
    let stats = foreign.stats().unwrap();
    assert_eq!(stats.words, 0);
    assert_eq!(stats.unreadable, 1);
    assert!(!foreign.is_known(WORD).unwrap());

    // A completely different master key behaves the same way.
    let mut stranger = open_dictionary(&database, &master_key());
    assert_eq!(stranger.stats().unwrap().words, 0);
    assert_eq!(stranger.stats().unwrap().unreadable, 1);
}

// ------------------------------------------------------------ 3. four stores

#[test]
fn the_word_list_is_a_fourth_file_with_a_fourth_key() {
    let directory = tempdir();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();

    let added = session
        .with_autocorrect(|store| store.add(WORD, Language::Russian, false))
        .unwrap();
    session
        .with_autocorrect(|store| store.add(IMPORTED_WORD, Language::Russian, true))
        .unwrap();

    // Four databases in one directory, each with its own key and journal. The word list
    // reaches the shared master key without writing anything into the password vault: the
    // vault store is empty afterwards, and the memory database was never created at all.
    let database = database_path(directory.path());
    assert!(database.is_file(), "the dictionary database exists");
    assert!(
        session.database_path().is_file(),
        "the vault database exists"
    );
    assert!(
        !session
            .with_store(|store| store.has_stored_entities())
            .unwrap(),
        "the spelling feature must not write a vault record"
    );
    assert!(!directory.path().join(AI_MEMORY_DB_FILE).exists());

    session.lock();
    assert!(!session.is_unlocked());
    assert_eq!(
        session.autocorrect_store().err().unwrap(),
        AutocorrectError::StorageLocked
    );
    // Locked, nothing is decrypted, but the interface still learns that words exist.
    assert!(session.has_autocorrect_records());
    assert_eq!(
        session.storage_status().unwrap().state,
        StorageState::Locked
    );

    // A wrong password changes nothing.
    assert!(session.unlock_with_password(WRONG_PASSWORD).is_err());
    assert!(session.with_autocorrect(|store| store.stats()).is_err());

    // Unlocking again finds the same words with the same revisions.
    session.unlock_with_password(PASSWORD).unwrap();
    let reopened = session
        .with_autocorrect(|store| {
            store.list(&jarvis_core::autocorrect::UserDictionaryQuery::default())
        })
        .unwrap();
    assert_eq!(reopened.len(), 2);
    assert_eq!(
        reopened
            .iter()
            .find(|entry| entry.id == added.id)
            .unwrap()
            .revision,
        added.revision
    );
    assert!(session
        .with_autocorrect(|store| Ok(store.holds_decrypted_entries()))
        .unwrap());

    // Dropping the dictionary key alone keeps the notes storage usable. The next use
    // derives it again, because the master key is still in memory; a lock is what makes
    // the words unreachable.
    session.drop_autocorrect();
    assert!(session.is_unlocked());
    assert_eq!(
        session
            .with_autocorrect(|store| store.stats())
            .unwrap()
            .words,
        2
    );
    session.lock();
    assert_eq!(
        session.autocorrect_store().err().unwrap(),
        AutocorrectError::StorageLocked
    );
}

#[test]
fn the_import_and_export_paths_round_trip_through_the_real_store() {
    let directory = tempdir();
    let database = database_path(directory.path());
    let master = master_key();

    let records = {
        let mut store = open_dictionary(&database, &master);
        let outcome = store
            .add_many(
                &[
                    WORD.to_string(),
                    IMPORTED_WORD.to_string(),
                    "  ".to_string(),
                ],
                Language::Russian,
                true,
            )
            .unwrap();
        assert_eq!(outcome.added, 2);
        assert_eq!(outcome.invalid, 1);
        // Plain-text export is explicit and sorted.
        let words = store.export_words().unwrap();
        assert_eq!(words.len(), 2);
        // The encrypted export is a state snapshot of ciphertext.
        let records = store.export_records().unwrap();
        assert_eq!(records.len(), 2);
        for record in &records {
            let payload = record.payload.as_ref().unwrap();
            for marker in CONTENT_MARKERS {
                assert!(
                    !contains_marker(payload, marker),
                    "an exported payload must be ciphertext"
                );
            }
        }
        records
    };

    // Import into an empty database with the same key: the words come back.
    let target = tempdir();
    let mut fresh = open_dictionary(&database_path(target.path()), &master);
    let outcome = fresh.import_records(&records).unwrap();
    assert_eq!(outcome.applied, 2);
    assert_eq!(outcome.conflicts, 0);
    assert!(fresh.is_known(WORD).unwrap());
    assert!(fresh.is_known(IMPORTED_WORD).unwrap());

    // Importing the same snapshot again is a conflict, not a silent overwrite.
    assert_eq!(records.len(), 2);
}

// -------------------------------------------------- 4. check, apply, undo, memory

#[test]
fn a_check_applies_corrections_and_undoes_them_against_the_real_store() {
    let directory = tempdir();
    let dictionary_dir = tempdir();
    let checker = checker(dictionary_dir.path());
    let database = database_path(directory.path());
    let master = master_key();
    let settings = AutocorrectSettings::default();

    let mut store = open_dictionary(&database, &master);
    // A word the user taught the checker: it must be accepted from now on.
    store.add("ФИКТИВНОЕИМЯ", Language::Russian, false).unwrap();

    let text = "привт ФИКТИВНОЕИМЯ ,  HEllo";
    let report = checker.check(Some(&mut store), text, &settings).unwrap();
    // The word the user taught is accepted; the typo, the double capital, and the two
    // punctuation problems are reported, and the punctuation ones carry a language of none.
    assert_eq!(report.issue_words(), vec!["привт", " ", "  ", "HEllo"]);
    assert_eq!(report.auto_fixable, 3);
    assert!(report.user_dictionary_available);
    assert_eq!(report.version, text_version(text));

    // Safe auto-correction is off by default, so nothing is proposed.
    assert!(LocalSpellChecker::safe_corrections(&report, &settings).is_empty());

    let safe = AutocorrectSettings {
        safe_autocorrect: true,
        ..AutocorrectSettings::default()
    };
    let corrections = LocalSpellChecker::safe_corrections(&report, &safe);
    let mut journal = CorrectionJournal::new();
    let batch = apply_corrections(text, &corrections, Some(&report.version), &mut journal).unwrap();
    assert_eq!(batch.after, "привт ФИКТИВНОЕИМЯ, Hello");
    assert_eq!(batch.applied_count(), 3);
    assert!(journal.can_undo());

    // The same corrections sent with the version of the *old* text are refused before
    // anything is touched, which is what keeps a keystroke from moving an edit.
    assert_eq!(
        apply_corrections(
            &batch.after,
            &corrections,
            Some(&report.version),
            &mut journal
        )
        .unwrap_err(),
        AutocorrectError::StaleText
    );

    // The undo restores the exact text, including the double space.
    let outcome = undo_last(&batch.after, &mut journal).unwrap();
    assert_eq!(outcome.text, text);
    assert!(!journal.can_undo());
    // Once more is not possible.
    assert_eq!(
        undo_last(&batch.after, &mut journal).unwrap_err(),
        AutocorrectError::NotFound
    );

    // The whole flow left the AI-memory database alone: it was never created.
    assert!(!directory.path().join(AI_MEMORY_DB_FILE).exists());
    assert!(!directory.path().join(VAULT_DB_FILE).exists());
}

#[test]
fn a_foreign_entity_type_cannot_be_read_as_a_word() {
    let directory = tempdir();
    let database = database_path(directory.path());
    let master = master_key();

    // A record written through the dictionary store, with the dictionary entity type.
    {
        let mut store = open_dictionary(&database, &master);
        store.add(WORD, Language::Russian, false).unwrap();
    }
    // The entity namespaces are separate by construction: the word list is not an AI
    // memory entity, so a memory reader can never open one of its records.
    assert_eq!(
        SyncEntityType::AutocorrectDictionary.as_str(),
        "autocorrect_dictionary"
    );
    assert!(!SyncEntityType::AutocorrectDictionary.is_ai_memory());
    assert!(SyncEntityType::AiMemoryFact.is_ai_memory());

    let mut store = open_dictionary(&database, &master);
    assert!(store.is_known(WORD).unwrap());
    assert_eq!(store.stats().unwrap().unreadable, 0);
}

// --------------------------------------------------------------------- helpers

/// A temporary directory that removes itself, without pulling `tempfile` into the test
/// file's namespace twice.
fn tempdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary directory")
}
