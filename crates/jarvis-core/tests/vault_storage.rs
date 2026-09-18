//! Integration tests for the encrypted password vault: lifecycle, restart,
//! master-password change, portable backup, and the guarantee that no secret
//! reaches disk, the write-ahead log, or diagnostics.
//!
//! Every fixture is fictional.

use jarvis_core::notes::StorageState;
use jarvis_core::vault::{
    change_master_password, VaultError, VaultItemDraft, VaultQuery, VaultSession, VaultSort,
    VaultTrashFilter, VAULT_DB_FILE,
};
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const PASSWORD: &str = "fictional-master-password";
const NEW_PASSWORD: &str = "fictional-new-master-password";
const WRONG_PASSWORD: &str = "fictional-wrong-password";

const SECRET_PASSWORD: &str = "FICTIONAL_ITEM_PASSWORD";
const SECRET_NOTES: &str = "FICTIONAL_ITEM_NOTES";
const SECRET_NAME: &str = "FICTIONAL_ITEM_NAME";
const SECRET_USERNAME: &str = "FICTIONAL_ITEM_USERNAME";
const SECRET_URL: &str = "https://fictional.example.com/private-path";
const SECRET_TAG: &str = "FICTIONAL_ITEM_TAG";

fn draft() -> VaultItemDraft {
    VaultItemDraft {
        name: SECRET_NAME.to_string(),
        username: SECRET_USERNAME.to_string(),
        password: SECRET_PASSWORD.to_string(),
        urls: vec![SECRET_URL.to_string()],
        notes: SECRET_NOTES.to_string(),
        tags: vec![SECRET_TAG.to_string()],
        favorite: false,
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack.windows(needle.len()).any(|window| window == needle)
}

/// Reads a SQLite database together with its write-ahead log.
fn database_bytes(path: &Path) -> Vec<u8> {
    let mut bytes = std::fs::read(path).unwrap_or_default();
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    bytes.extend(std::fs::read(PathBuf::from(wal)).unwrap_or_default());
    bytes
}

#[test]
fn a_vault_session_locks_unlocks_and_keeps_items_across_restarts() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();

    let status = session.status().unwrap();
    assert_eq!(status.storage.state, StorageState::Uninitialized);
    assert!(!status.is_unlocked());
    assert_eq!(status.stats.items_total, 0);
    // The vault keeps its own database, next to the notes database.
    assert_eq!(
        session.database_path(),
        directory.path().join(VAULT_DB_FILE)
    );

    let status = session.initialize(PASSWORD).unwrap();
    assert!(status.is_unlocked());
    assert_eq!(status.stats.items_total, 0);

    let item = session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();
    assert_eq!(item.revision, 1);
    assert_eq!(session.status().unwrap().stats.items_total, 1);

    // While locked, no secret can be produced and the cache is gone.
    session.lock();
    assert!(!session.is_unlocked());
    assert_eq!(session.store().err().unwrap(), VaultError::StorageLocked);
    assert_eq!(
        session.reveal_secret(item.id, 30).err().unwrap(),
        VaultError::StorageLocked
    );
    let locked_status = session.status().unwrap();
    assert_eq!(locked_status.storage.state, StorageState::Locked);
    assert_eq!(locked_status.stats.items_total, 0);

    // A wrong password leaves the vault locked.
    assert!(session.unlock_with_password(WRONG_PASSWORD).is_err());
    assert!(!session.is_unlocked());

    // The right password restores the item.
    let status = session.unlock_with_password(PASSWORD).unwrap();
    assert!(status.is_unlocked());
    assert_eq!(status.stats.items_total, 1);
    let revealed = session.reveal_secret(item.id, 30).unwrap();
    assert_eq!(revealed.password, SECRET_PASSWORD);
    assert_eq!(revealed.notes, SECRET_NOTES);
    assert!(!session
        .with_store(|store| store.get_item(item.id))
        .unwrap()
        .unwrap()
        .favorite);

    // A fresh session object (application restart) behaves the same.
    drop(session);
    let mut restarted = VaultSession::open(directory.path()).unwrap();
    assert_eq!(
        restarted.status().unwrap().storage.state,
        StorageState::Locked
    );
    restarted.unlock_with_password(PASSWORD).unwrap();
    let list = restarted
        .with_store(|store| store.list_items(&VaultQuery::default()))
        .unwrap();
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].url_host.as_deref(), Some("fictional.example.com"));
}

#[test]
fn trash_favorite_and_permanent_deletion_work_through_the_session() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();

    let item = session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();
    session
        .with_store(|store| store.set_favorite(item.id, true))
        .unwrap();
    assert_eq!(session.status().unwrap().stats.favorites, 1);

    session
        .with_store(|store| store.trash_item(item.id))
        .unwrap();
    let trashed = session
        .with_store(|store| {
            store.list_items(&VaultQuery {
                trash: VaultTrashFilter::Trashed,
                ..VaultQuery::default()
            })
        })
        .unwrap();
    assert_eq!(trashed.items.len(), 1);
    assert_eq!(session.status().unwrap().stats.items_trashed, 1);

    session
        .with_store(|store| store.restore_item(item.id))
        .unwrap();
    assert_eq!(session.status().unwrap().stats.items_trashed, 0);

    session
        .with_store(|store| store.purge_item(item.id))
        .unwrap();
    assert_eq!(session.status().unwrap().stats.items_total, 0);
    // The tombstone is durable: a restart must not bring it back.
    session.lock();
    session.unlock_with_password(PASSWORD).unwrap();
    assert!(session
        .with_store(|store| store.get_item(item.id))
        .unwrap()
        .is_none());
}

#[test]
fn secret_content_never_reaches_the_database_the_wal_or_diagnostics() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();

    let item = session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();
    let details = session
        .with_store(|store| store.get_item(item.id))
        .unwrap()
        .unwrap();
    let list = session
        .with_store(|store| store.list_items(&VaultQuery::default()))
        .unwrap();
    let revealed = session.reveal_secret(item.id, 30).unwrap();

    // The DTOs must not print secrets, and the details DTO has no password at
    // all (the struct cannot carry one).
    let rendered = format!("{details:?} {list:?} {revealed:?} {:?}", list.items[0]);
    for marker in [SECRET_PASSWORD, SECRET_NOTES, SECRET_NAME] {
        assert!(
            !rendered.contains(marker),
            "{marker} leaked into diagnostics"
        );
    }
    assert!(rendered.contains("<redacted>"));

    // Lock, so the cache is dropped and everything is on disk.
    session.lock();
    drop(session);

    let stored = database_bytes(&directory.path().join(VAULT_DB_FILE));
    assert!(!stored.is_empty(), "the vault database must exist");
    for marker in [
        SECRET_PASSWORD,
        SECRET_NOTES,
        SECRET_NAME,
        SECRET_USERNAME,
        SECRET_URL,
        SECRET_TAG,
    ] {
        assert!(
            !contains(&stored, marker.as_bytes()),
            "{marker} reached the vault database or its write-ahead log"
        );
    }

    // The portable envelope never contains the master password either.
    let envelope =
        std::fs::read_to_string(directory.path().join("key.backup.json")).unwrap();
    assert!(!envelope.contains(PASSWORD));
    assert!(!envelope.contains(SECRET_PASSWORD));
}

#[test]
fn a_different_master_key_cannot_read_the_vault() {
    // Source installation: its own master key and one stored item.
    let source = tempdir().unwrap();
    let mut session = VaultSession::open(source.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    let item = session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();
    session.lock();
    drop(session);

    // Target installation: a different master key.
    let target = tempdir().unwrap();
    let mut foreign = VaultSession::open(target.path()).unwrap();
    foreign.initialize(WRONG_PASSWORD).unwrap();
    foreign.lock();
    drop(foreign);

    // Drop the source database into the target installation, so the key no
    // longer matches the data on disk.
    std::fs::copy(
        source.path().join(VAULT_DB_FILE),
        target.path().join(VAULT_DB_FILE),
    )
    .unwrap();

    let mut mismatched = VaultSession::open(target.path()).unwrap();
    mismatched.unlock_with_password(WRONG_PASSWORD).unwrap();
    assert_eq!(
        mismatched.with_store(|store| store.get_item(item.id)),
        Err(VaultError::Unreadable)
    );
    let list = mismatched
        .with_store(|store| {
            store.list_items(&VaultQuery {
                trash: VaultTrashFilter::All,
                ..VaultQuery::default()
            })
        })
        .unwrap();
    assert!(list.items.is_empty());
    assert_eq!(list.unreadable, 1);
}

#[test]
fn a_portable_backup_restores_the_vault_in_a_new_data_directory() {
    let source = tempdir().unwrap();
    let mut session = VaultSession::open(source.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    let item = session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();

    // Export a fresh envelope with its own password, as the interface does.
    let export_dir = tempdir().unwrap();
    let destination = export_dir.path().join("jarvis-key-backup.json");
    session
        .export_backup_to(NEW_PASSWORD, &destination)
        .unwrap();
    let envelope = std::fs::read_to_string(&destination).unwrap();
    assert!(envelope.contains("argon2id"));
    assert!(!envelope.contains(NEW_PASSWORD));
    session.lock();
    drop(session);

    // New machine: copy the encrypted database, no key files at all.
    let target = tempdir().unwrap();
    std::fs::copy(
        source.path().join(VAULT_DB_FILE),
        target.path().join(VAULT_DB_FILE),
    )
    .unwrap();
    let mut fresh = VaultSession::open(target.path()).unwrap();
    assert_eq!(
        fresh.status().unwrap().storage.state,
        StorageState::KeyMissing
    );
    // A wrong password for the imported envelope changes nothing.
    assert!(fresh.import_backup(&envelope, WRONG_PASSWORD).is_err());
    assert!(!fresh.is_unlocked());

    let status = fresh.import_backup(&envelope, NEW_PASSWORD).unwrap();
    assert!(status.is_unlocked());
    assert_eq!(status.stats.items_total, 1);
    assert_eq!(
        fresh.reveal_secret(item.id, 30).unwrap().password,
        SECRET_PASSWORD
    );
}

#[test]
fn changing_the_master_password_rewraps_the_key_without_touching_the_data() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    let item = session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();
    let revision_before = item.revision;

    let change = session.change_master_password(PASSWORD, NEW_PASSWORD).unwrap();
    assert!(change.backup_replaced);
    assert_eq!(change.dpapi_updated, cfg!(windows));

    // The change must not re-encrypt anything: same item, same revision.
    let details = session
        .with_store(|store| store.get_item(item.id))
        .unwrap()
        .unwrap();
    assert_eq!(details.revision, revision_before);

    // After a restart the new password unlocks and the old one does not.
    session.lock();
    drop(session);
    let mut restarted = VaultSession::open(directory.path()).unwrap();
    assert!(restarted.unlock_with_password(PASSWORD).is_err());
    restarted.unlock_with_password(NEW_PASSWORD).unwrap();
    assert_eq!(
        restarted.reveal_secret(item.id, 30).unwrap().password,
        SECRET_PASSWORD
    );

    // The envelope on disk now answers to the new password only.
    let envelope =
        std::fs::read_to_string(directory.path().join("key.backup.json")).unwrap();
    assert!(!envelope.contains(NEW_PASSWORD));
    let parsed = jarvis_core::sync::crypto::PortableKeyBackup::from_json(&envelope).unwrap();
    assert!(jarvis_core::sync::crypto::import_backup(&parsed, PASSWORD.as_bytes()).is_err());
    assert!(jarvis_core::sync::crypto::import_backup(&parsed, NEW_PASSWORD.as_bytes()).is_ok());
}

#[test]
fn a_failed_master_password_change_leaves_the_old_password_working() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    let item = session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();

    // Wrong current password: nothing may change.
    assert!(session
        .change_master_password(WRONG_PASSWORD, NEW_PASSWORD)
        .is_err());
    // Too short new password: refused before any file is written.
    assert!(session
        .change_master_password(PASSWORD, "short")
        .is_err());
    // Unchanged password: refused, because a no-op change would be misleading.
    assert!(session
        .change_master_password(PASSWORD, PASSWORD)
        .is_err());

    let envelope =
        std::fs::read_to_string(directory.path().join("key.backup.json")).unwrap();
    let parsed = jarvis_core::sync::crypto::PortableKeyBackup::from_json(&envelope).unwrap();
    assert!(
        jarvis_core::sync::crypto::import_backup(&parsed, PASSWORD.as_bytes()).is_ok(),
        "the original envelope must remain valid"
    );
    assert!(
        jarvis_core::sync::crypto::import_backup(&parsed, NEW_PASSWORD.as_bytes()).is_err(),
        "the refused new password must not work"
    );

    session.lock();
    session.unlock_with_password(PASSWORD).unwrap();
    assert_eq!(
        session.reveal_secret(item.id, 30).unwrap().password,
        SECRET_PASSWORD
    );
}

#[test]
fn the_free_function_change_helper_requires_the_current_password() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    let storage = session.storage_mut();
    assert!(change_master_password(storage, WRONG_PASSWORD, NEW_PASSWORD).is_err());
    assert!(change_master_password(storage, PASSWORD, NEW_PASSWORD).is_ok());
}

#[test]
fn vault_and_notes_databases_stay_separate_files() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();
    session.lock();
    drop(session);

    let notes = directory.path().join("sync.sqlite3");
    let vault = directory.path().join(VAULT_DB_FILE);
    assert!(notes.is_file());
    assert!(vault.is_file());
    assert_ne!(notes, vault);

    // The vault database must not contain note entity types and the notes
    // database must not contain vault records.
    let vault_bytes = database_bytes(&vault);
    assert!(!contains(&vault_bytes, b"note_folder"));
    let notes_bytes = database_bytes(&notes);
    assert!(!contains(&notes_bytes, b"vault_record"));
    // The vault journal does carry its own entity type.
    assert!(contains(&vault_bytes, b"vault_record"));
}

#[cfg(windows)]
#[test]
fn dpapi_still_unlocks_after_a_master_password_change() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    let item = session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();
    let before = std::fs::read(directory.path().join("key.dpapi")).unwrap();

    session.change_master_password(PASSWORD, NEW_PASSWORD).unwrap();
    let after = std::fs::read(directory.path().join("key.dpapi")).unwrap();
    assert_ne!(before, after, "the DPAPI blob must be refreshed");

    session.lock();
    let status = session.unlock_with_dpapi().unwrap();
    assert!(status.is_unlocked());
    assert_eq!(
        session.reveal_secret(item.id, 30).unwrap().password,
        SECRET_PASSWORD
    );
}

#[cfg(windows)]
#[test]
fn dpapi_unlock_works_after_a_restart() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    session
        .with_store(|store| store.create_item(&draft()))
        .unwrap();
    session.lock();
    drop(session);

    let mut restarted = VaultSession::open(directory.path()).unwrap();
    assert_eq!(
        restarted.status().unwrap().storage.state,
        StorageState::Locked
    );
    restarted.unlock_with_dpapi().unwrap();
    let list = restarted
        .with_store(|store| store.list_items(&VaultQuery::default()))
        .unwrap();
    assert_eq!(list.items.len(), 1);
}

#[cfg(not(windows))]
#[test]
fn dpapi_reports_an_unsupported_platform_for_the_vault() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();
    session.lock();
    assert!(session.unlock_with_dpapi().is_err());
    assert!(session.unlock_with_password(PASSWORD).is_ok());
}

#[test]
fn generated_passwords_can_be_stored_and_revealed() {
    let directory = tempdir().unwrap();
    let mut session = VaultSession::open(directory.path()).unwrap();
    session.initialize(PASSWORD).unwrap();

    let policy = jarvis_core::vault::PasswordPolicy::default();
    assert_eq!(policy.length, jarvis_core::vault::DEFAULT_LENGTH);
    let generated = jarvis_core::vault::generate_password(&policy).unwrap();
    let mut item_draft = draft();
    item_draft.password = generated.clone();
    let item = session
        .with_store(|store| store.create_item(&item_draft))
        .unwrap();

    // Sorting by name must keep the list stable and secret-free.
    let list = session
        .with_store(|store| {
            store.list_items(&VaultQuery {
                sort: VaultSort::NameAsc,
                ..VaultQuery::default()
            })
        })
        .unwrap();
    assert_eq!(list.items.len(), 1);
    assert_eq!(
        session.reveal_secret(item.id, 30).unwrap().password,
        generated
    );
}
