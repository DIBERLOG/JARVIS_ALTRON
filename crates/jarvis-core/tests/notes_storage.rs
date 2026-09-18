//! Integration tests for the encrypted notes vault: the full key lifecycle,
//! persistence across restarts, and the guarantee that no plaintext note ever
//! reaches disk.
//!
//! Every fixture is fictional.

use jarvis_core::notes::{
    NoteDraft, NoteError, NoteQuery, NotesVault, StorageState, TrashFilter, BACKUP_KEY_FILE,
    DEVICE_ID_FILE, DPAPI_KEY_FILE,
};
use std::path::{Path, PathBuf};
use tempfile::tempdir;

const PASSWORD: &str = "fictional-master-password";
const WRONG_PASSWORD: &str = "fictional-wrong-password";

fn draft(title: &str, body: &str) -> NoteDraft {
    NoteDraft {
        title: title.to_string(),
        body: body.to_string(),
        folder_id: None,
        tags: Vec::new(),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack.windows(needle.len()).any(|window| window == needle)
}

/// Reads the database plus its write-ahead log, so a scan cannot miss data that
/// has not been checkpointed yet.
fn database_bytes(data_dir: &Path) -> Vec<u8> {
    let mut bytes = std::fs::read(data_dir.join("sync.sqlite3")).unwrap_or_default();
    let mut wal = data_dir.join("sync.sqlite3").into_os_string();
    wal.push("-wal");
    bytes.extend(std::fs::read(PathBuf::from(wal)).unwrap_or_default());
    bytes
}

#[test]
fn initializes_unlocks_relocks_and_persists() {
    let directory = tempdir().unwrap();
    let mut vault = NotesVault::open(directory.path()).unwrap();

    let status = vault.status().unwrap();
    assert_eq!(status.state, StorageState::Uninitialized);
    assert!(!status.has_stored_data);
    assert!(!status.dpapi_available);
    assert!(!status.backup_available);
    assert_eq!(status.stats.notes_total, 0);

    let status = vault.initialize(PASSWORD).unwrap();
    assert_eq!(status.state, StorageState::Unlocked);
    assert!(status.backup_available);
    assert!(directory.path().join(BACKUP_KEY_FILE).is_file());
    assert!(directory.path().join(DEVICE_ID_FILE).is_file());
    // DPAPI is a Windows-only convenience; the portable envelope always exists.
    assert_eq!(
        directory.path().join(DPAPI_KEY_FILE).is_file(),
        cfg!(windows)
    );

    let note_id = vault
        .with_store(|store| Ok(store.create_note(&draft("FICTIONAL_TITLE", "FICTIONAL_BODY"))?.id))
        .unwrap();
    vault
        .with_store(|store| {
            store.create_note(&draft("FICTIONAL_SECOND", "second body"))?;
            Ok(())
        })
        .unwrap();
    assert_eq!(vault.status().unwrap().stats.notes_total, 2);

    // Locking drops the key: no content may be produced any more.
    vault.lock();
    let locked = vault.status().unwrap();
    assert_eq!(locked.state, StorageState::Locked);
    assert_eq!(locked.stats.notes_total, 0);
    assert_eq!(vault.store().err().unwrap(), NoteError::StorageLocked);
    assert_eq!(
        vault
            .with_store(|store| Ok(store.list_notes(&NoteQuery::default())?.items.len()))
            .err()
            .unwrap(),
        NoteError::StorageLocked
    );

    // A wrong master password is refused and keeps the vault locked.
    assert!(vault.unlock_with_password(WRONG_PASSWORD).is_err());
    assert!(!vault.is_unlocked());
    assert_eq!(vault.status().unwrap().state, StorageState::Locked);

    // The right password restores access to the same content.
    let status = vault.unlock_with_password(PASSWORD).unwrap();
    assert_eq!(status.state, StorageState::Unlocked);
    assert_eq!(status.stats.notes_total, 2);
    let note = vault
        .with_store(|store| Ok(store.get_note(note_id)?.unwrap()))
        .unwrap();
    assert_eq!(note.title, "FICTIONAL_TITLE");
    assert_eq!(note.body, "FICTIONAL_BODY");
    assert_eq!(note.revision, 1);

    // Re-initializing an existing storage must never orphan the key.
    assert_eq!(
        vault.initialize(PASSWORD).err().unwrap(),
        NoteError::AlreadyInitialized
    );

    // A restart (fresh vault object) starts locked and reads the same data.
    drop(vault);
    let mut restarted = NotesVault::open(directory.path()).unwrap();
    assert_eq!(restarted.status().unwrap().state, StorageState::Locked);
    restarted.unlock_with_password(PASSWORD).unwrap();
    assert_eq!(
        restarted
            .with_store(|store| Ok(store.get_note(note_id)?.unwrap().body))
            .unwrap(),
        "FICTIONAL_BODY"
    );
}

#[test]
fn trash_and_permanent_deletion_survive_a_restart() {
    let directory = tempdir().unwrap();
    let mut vault = NotesVault::open(directory.path()).unwrap();
    vault.initialize(PASSWORD).unwrap();

    let trashed_id = vault
        .with_store(|store| Ok(store.create_note(&draft("FICTIONAL_TRASH", "body"))?.id))
        .unwrap();
    let purged_id = vault
        .with_store(|store| Ok(store.create_note(&draft("FICTIONAL_PURGE", "body"))?.id))
        .unwrap();

    vault
        .with_store(|store| {
            store.trash_note(trashed_id)?;
            store.purge_note(purged_id)?;
            Ok(())
        })
        .unwrap();

    drop(vault);
    let mut restarted = NotesVault::open(directory.path()).unwrap();
    restarted.unlock_with_password(PASSWORD).unwrap();

    let trashed = restarted
        .with_store(|store| {
            Ok(store
                .list_notes(&NoteQuery {
                    trash: TrashFilter::Trashed,
                    ..NoteQuery::default()
                })?
                .items
                .len())
        })
        .unwrap();
    assert_eq!(trashed, 1);

    // The purged note is gone in every view, including the trash.
    assert!(restarted
        .with_store(|store| Ok(store.get_note(purged_id)?.is_none()))
        .unwrap());
    let all = restarted
        .with_store(|store| {
            Ok(store
                .list_notes(&NoteQuery {
                    trash: TrashFilter::All,
                    ..NoteQuery::default()
                })?
                .items
                .len())
        })
        .unwrap();
    assert_eq!(all, 1);

    // Restoring brings the trashed note back.
    restarted
        .with_store(|store| {
            store.restore_note(trashed_id)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(restarted.status().unwrap().stats.notes_total, 1);
    assert_eq!(restarted.status().unwrap().stats.notes_trashed, 0);
}

#[test]
fn note_and_folder_content_never_reaches_disk_or_logs() {
    let directory = tempdir().unwrap();
    let mut vault = NotesVault::open(directory.path()).unwrap();
    vault.initialize(PASSWORD).unwrap();

    vault
        .with_store(|store| {
            let mut assignment = draft("FICTIONAL_TITLE_MARKER", "FICTIONAL_BODY_MARKER");
            assignment.tags = vec!["FICTIONAL_TAG_MARKER".into()];
            store.create_note(&assignment)?;
            store.create_folder("FICTIONAL_FOLDER_MARKER")?;
            Ok(())
        })
        .unwrap();

    let stored = vault
        .with_store(|store| Ok(store.decrypt_note_set()?.len()))
        .unwrap();
    assert_eq!(stored, 1);

    drop(vault);

    let database = database_bytes(directory.path());
    assert!(!database.is_empty());
    for marker in [
        b"FICTIONAL_TITLE_MARKER".as_slice(),
        b"FICTIONAL_BODY_MARKER".as_slice(),
        b"FICTIONAL_TAG_MARKER".as_slice(),
        b"FICTIONAL_FOLDER_MARKER".as_slice(),
    ] {
        assert!(
            !contains(&database, marker),
            "plaintext {marker:?} reached the database files"
        );
    }

    // The key files never hold the master password or any note content.
    let envelope = std::fs::read_to_string(directory.path().join(BACKUP_KEY_FILE)).unwrap();
    assert!(!envelope.contains(PASSWORD));
    assert!(!envelope.contains("FICTIONAL"));
    let device = std::fs::read_to_string(directory.path().join(DEVICE_ID_FILE)).unwrap();
    assert!(!device.contains("FICTIONAL"));
}

#[test]
fn a_portable_backup_recovers_notes_in_a_new_data_directory() {
    let source = tempdir().unwrap();
    let mut vault = NotesVault::open(source.path()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    let note_id = vault
        .with_store(|store| Ok(store.create_note(&draft("FICTIONAL_PORTABLE", "portable body"))?.id))
        .unwrap();
    let envelope = vault.export_backup_json(PASSWORD).unwrap();
    assert!(envelope.contains("argon2id"));
    assert!(!envelope.contains(PASSWORD));
    drop(vault);

    // Simulate a new machine: the encrypted database is carried over, but no key
    // file is, so only the portable envelope can recover the data.
    let target = tempdir().unwrap();
    std::fs::copy(
        source.path().join("sync.sqlite3"),
        target.path().join("sync.sqlite3"),
    )
    .unwrap();
    let mut fresh = NotesVault::open(target.path()).unwrap();
    assert_eq!(fresh.status().unwrap().state, StorageState::KeyMissing);
    // Creating a new key here would orphan the existing data, so it is refused.
    assert_eq!(
        fresh.initialize(PASSWORD).err().unwrap(),
        NoteError::KeyMissing
    );
    assert!(fresh.unlock_with_password(PASSWORD).is_err());
    // A wrong password for the imported envelope changes nothing.
    assert!(fresh.import_backup(&envelope, WRONG_PASSWORD).is_err());
    assert!(!fresh.is_unlocked());

    let status = fresh.import_backup(&envelope, PASSWORD).unwrap();
    assert_eq!(status.state, StorageState::Unlocked);
    assert_eq!(
        fresh
            .with_store(|store| Ok(store.get_note(note_id)?.unwrap().body))
            .unwrap(),
        "portable body"
    );
    // The import re-creates the local key files for the next start.
    assert!(target.path().join(BACKUP_KEY_FILE).is_file());
    assert_eq!(
        target.path().join(DPAPI_KEY_FILE).is_file(),
        cfg!(windows)
    );

    // And the restored key keeps working after a restart.
    drop(fresh);
    let mut restarted = NotesVault::open(target.path()).unwrap();
    assert_eq!(
        restarted.unlock_with_password(PASSWORD).unwrap().state,
        StorageState::Unlocked
    );
    assert_eq!(restarted.status().unwrap().stats.notes_total, 1);
}

#[test]
fn a_copy_of_the_portable_envelope_can_be_saved_next_to_the_database() {
    let directory = tempdir().unwrap();
    let export_dir = tempdir().unwrap();
    let mut vault = NotesVault::open(directory.path()).unwrap();
    vault.initialize(PASSWORD).unwrap();

    let destination = export_dir.path().join("jarvis-key-backup.json");
    let written = vault.copy_local_backup_to(&destination).unwrap();
    assert_eq!(written, destination);
    let copied = std::fs::read_to_string(&destination).unwrap();
    let original =
        std::fs::read_to_string(directory.path().join(BACKUP_KEY_FILE)).unwrap();
    assert_eq!(copied, original);

    // A freshly written envelope must also be importable into another vault.
    let other = tempdir().unwrap();
    let mut other_vault = NotesVault::open(other.path()).unwrap();
    assert_eq!(
        other_vault.import_backup(&copied, PASSWORD).unwrap().state,
        StorageState::Unlocked
    );
}

#[cfg(windows)]
#[test]
fn dpapi_unlock_reaches_the_same_notes() {
    let directory = tempdir().unwrap();
    let mut vault = NotesVault::open(directory.path()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    let note_id = vault
        .with_store(|store| Ok(store.create_note(&draft("FICTIONAL_DPAPI", "dpapi body"))?.id))
        .unwrap();
    vault.lock();

    let status = vault.unlock_with_dpapi().unwrap();
    assert_eq!(status.state, StorageState::Unlocked);
    assert!(status.dpapi_available);
    assert_eq!(
        vault
            .with_store(|store| Ok(store.get_note(note_id)?.unwrap().body))
            .unwrap(),
        "dpapi body"
    );

    // A damaged DPAPI blob must not unlock, and must not lose the password path.
    vault.lock();
    let blob_path = directory.path().join(DPAPI_KEY_FILE);
    let mut blob = std::fs::read(&blob_path).unwrap();
    let last = blob.len() - 1;
    blob[last] ^= 0x5a;
    std::fs::write(&blob_path, &blob).unwrap();
    assert!(vault.unlock_with_dpapi().is_err());
    assert!(!vault.is_unlocked());
    assert_eq!(
        vault.unlock_with_password(PASSWORD).unwrap().state,
        StorageState::Unlocked
    );
}

#[cfg(windows)]
#[test]
fn a_restored_backup_replaces_the_local_dpapi_key() {
    let directory = tempdir().unwrap();
    let mut vault = NotesVault::open(directory.path()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    let note_id = vault
        .with_store(|store| Ok(store.create_note(&draft("FICTIONAL_RESTORE", "restore body"))?.id))
        .unwrap();
    let envelope = vault.export_backup_json(PASSWORD).unwrap();
    let original_blob = std::fs::read(directory.path().join(DPAPI_KEY_FILE)).unwrap();
    drop(vault);

    // A fresh vault with a brand new local key: importing the portable envelope
    // must switch it to the restored master key.
    let mut second = NotesVault::open(directory.path()).unwrap();
    let replace_directory = tempdir().unwrap();
    let mut other = NotesVault::open(replace_directory.path()).unwrap();
    other.initialize("fictional-other-master").unwrap();
    let other_blob = std::fs::read(replace_directory.path().join(DPAPI_KEY_FILE)).unwrap();
    assert_ne!(original_blob, other_blob);

    second.import_backup(&envelope, PASSWORD).unwrap();
    assert_eq!(
        second
            .with_store(|store| Ok(store.get_note(note_id)?.unwrap().body))
            .unwrap(),
        "restore body"
    );
    let restored_blob = std::fs::read(directory.path().join(DPAPI_KEY_FILE)).unwrap();
    assert_ne!(restored_blob, other_blob);
    second.lock();
    assert_eq!(
        second.unlock_with_dpapi().unwrap().state,
        StorageState::Unlocked
    );
}

#[cfg(not(windows))]
#[test]
fn dpapi_unlock_reports_an_unsupported_platform() {
    let directory = tempdir().unwrap();
    let mut vault = NotesVault::open(directory.path()).unwrap();
    vault.initialize(PASSWORD).unwrap();
    vault.lock();
    assert!(vault.unlock_with_dpapi().is_err());
    assert_eq!(
        vault.unlock_with_password(PASSWORD).unwrap().state,
        StorageState::Unlocked
    );
}
