//! End-to-end tests for the durable SQLite sync repository.
//!
//! Every fixture is fictional. Payloads are produced by the real crypto layer so
//! the storage path is exercised exactly as production uses it.

use jarvis_core::sync::crypto::{random_master_key, MasterKeyCryptoProvider};
use jarvis_core::sync::sqlite::{SqliteSyncRepository, SCHEMA_VERSION};
use jarvis_core::sync::{
    ApplyOutcome, ConflictReason, CryptoProvider, DeviceId, MutationConflict, PayloadContext,
    StoredOperationOutcome, SyncCursor, SyncEntityType, SyncError, SyncMutation,
    SyncOperationKind, SyncRepository,
};use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::tempdir;
use uuid::Uuid;

const DEVICE: &str = "sqlite_test_device";
const ENTITY_TYPE: SyncEntityType = SyncEntityType::Note;

/// Oldest schema this repository still migrates from. Frozen on purpose.
const SCHEMA_V1_FIXTURE: &str = "
CREATE TABLE sync_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE sync_entities (
    entity_id         TEXT PRIMARY KEY,
    entity_type       TEXT NOT NULL,
    entity_revision   INTEGER NOT NULL,
    tombstone         INTEGER NOT NULL,
    payload           BLOB,
    last_operation_id TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE TABLE sync_operations (
    server_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id    TEXT NOT NULL UNIQUE,
    entity_id       TEXT NOT NULL,
    entity_type     TEXT NOT NULL,
    device_id       TEXT NOT NULL,
    device_sequence INTEGER NOT NULL,
    base_revision   INTEGER NOT NULL,
    entity_revision INTEGER NOT NULL,
    operation_kind  TEXT NOT NULL,
    tombstone       INTEGER NOT NULL,
    payload         BLOB,
    schema_version  INTEGER NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE(device_id, device_sequence)
);
CREATE TABLE sync_conflicts (
    conflict_id     TEXT PRIMARY KEY,
    entity_id       TEXT NOT NULL,
    entity_type     TEXT NOT NULL,
    current_revision INTEGER NOT NULL,
    current_payload BLOB,
    incoming_payload BLOB,
    reason          TEXT NOT NULL,
    detected_at     TEXT NOT NULL,
    device_id       TEXT NOT NULL,
    operation_id    TEXT NOT NULL UNIQUE
);
CREATE TABLE sync_devices (device_id TEXT PRIMARY KEY);
CREATE TABLE sync_device_cursors (
    device_id           TEXT PRIMARY KEY REFERENCES sync_devices(device_id),
    last_server_sequence INTEGER NOT NULL DEFAULT 0
);
";

fn device() -> DeviceId {
    DeviceId::new(DEVICE).unwrap()
}

/// Builds a client mutation; the payload is real ciphertext.
fn mutation(
    provider: &MasterKeyCryptoProvider,
    operation_id: Uuid,
    entity_id: Uuid,
    device_sequence: u64,
    base_revision: u64,
    kind: SyncOperationKind,
    plaintext: Option<&[u8]>,
) -> SyncMutation {
    let encrypted_payload = plaintext.map(|plaintext| {
        provider
            .encrypt(
                &PayloadContext::new(ENTITY_TYPE, entity_id),
                plaintext,
            )
            .expect("fixture encryption must succeed")
    });
    SyncMutation {
        operation_id,
        entity_id,
        entity_type: ENTITY_TYPE,
        device_id: device(),
        device_sequence,
        base_revision,
        kind,
        timestamp: "2026-01-01T00:00:00Z".to_string(),
        schema_version: SyncMutation::CURRENT_SCHEMA_VERSION,
        encrypted_payload,
    }
}

fn create(
    provider: &MasterKeyCryptoProvider,
    entity_id: Uuid,
    sequence: u64,
    plaintext: &[u8],
) -> SyncMutation {
    mutation(
        provider,
        Uuid::new_v4(),
        entity_id,
        sequence,
        0,
        SyncOperationKind::Create,
        Some(plaintext),
    )
}

fn update(
    provider: &MasterKeyCryptoProvider,
    entity_id: Uuid,
    sequence: u64,
    base_revision: u64,
    plaintext: &[u8],
) -> SyncMutation {
    mutation(
        provider,
        Uuid::new_v4(),
        entity_id,
        sequence,
        base_revision,
        SyncOperationKind::Update,
        Some(plaintext),
    )
}

fn delete(
    provider: &MasterKeyCryptoProvider,
    entity_id: Uuid,
    sequence: u64,
    base_revision: u64,
) -> SyncMutation {
    mutation(
        provider,
        Uuid::new_v4(),
        entity_id,
        sequence,
        base_revision,
        SyncOperationKind::Delete,
        None,
    )
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack.len() >= needle.len()
        && haystack.windows(needle.len()).any(|window| window == needle)
}

fn database_bytes(path: &Path) -> Vec<u8> {
    let mut bytes = std::fs::read(path).unwrap_or_default();
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    bytes.extend(std::fs::read(PathBuf::from(wal)).unwrap_or_default());
    bytes
}

#[test]
fn creates_an_empty_database_and_has_the_expected_durable_pragmas() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("fresh.sqlite3");
    let repository = SqliteSyncRepository::open(&path).unwrap();

    assert!(path.exists());
    assert_eq!(repository.schema_version().unwrap(), SCHEMA_VERSION);
    assert!(repository.record(Uuid::new_v4()).unwrap().is_none());
    assert!(repository.conflicts().unwrap().is_empty());
    assert_eq!(
        repository.device_cursor(&device()).unwrap(),
        SyncCursor(0)
    );
    assert_eq!(repository.last_device_sequence(&device()).unwrap(), 0);

    let wal: String = repository_journal_mode(&path);
    assert_eq!(wal.to_ascii_lowercase(), "wal");
}

/// Reads the persisted journal mode through an independent connection.
fn repository_journal_mode(path: &Path) -> String {
    let connection = Connection::open(path).unwrap();
    connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn data_and_idempotency_survive_a_restart() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("restart.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let entity = Uuid::new_v4();
    let first = create(&provider, entity, 1, b"fictional note v1");

    {
        let mut repository = SqliteSyncRepository::open(&path).unwrap();
        assert_eq!(
            repository.apply_mutation(&first).unwrap(),
            ApplyOutcome::Applied {
                cursor: SyncCursor(1)
            }
        );
        repository
            .save_device_cursor(&device(), SyncCursor(1))
            .unwrap();
        repository.checkpoint_and_close().unwrap();
    }

    let mut repository = SqliteSyncRepository::open(&path).unwrap();
    let record = repository.record(entity).unwrap().unwrap();
    assert_eq!(record.metadata.revision, 1);
    assert_eq!(record.metadata.device_id.as_str(), DEVICE);
    assert!(!record.metadata.tombstone);
    let payload = record.content.clone().unwrap();
    assert_eq!(
        provider
            .decrypt(&PayloadContext::new(ENTITY_TYPE, entity), &payload)
            .unwrap(),
        b"fictional note v1"
    );

    // The same operation ID after a restart is still idempotent.
    assert_eq!(
        repository.apply_mutation(&first).unwrap(),
        ApplyOutcome::AlreadyApplied {
            cursor: SyncCursor(1)
        }
    );
    assert_eq!(repository.device_cursor(&device()).unwrap(), SyncCursor(1));
    assert_eq!(repository.last_device_sequence(&device()).unwrap(), 1);
    assert_eq!(
        repository.page_after(SyncCursor(0), 10).unwrap().operations.len(),
        1
    );
}

#[test]
fn journal_pages_respect_the_limit_and_advance_the_cursor() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("pages.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let mut repository = SqliteSyncRepository::open(&path).unwrap();

    for (index, entity) in (0..3).map(|_| Uuid::new_v4()).enumerate() {
        repository
            .apply_mutation(&create(
                &provider,
                entity,
                (index + 1) as u64,
                b"fictional",
            ))
            .unwrap();
    }

    let first = repository.page_after(SyncCursor(0), 2).unwrap();
    assert_eq!(first.operations.len(), 2);
    assert_eq!(first.next_cursor, SyncCursor(2));
    assert_eq!(first.operations[0].server_sequence, SyncCursor(1));
    assert_eq!(first.operations[1].entity_revision, 1);
    assert_eq!(first.operations[0].outcome, StoredOperationOutcome::Applied);

    let second = repository.page_after(first.next_cursor, 2).unwrap();
    assert_eq!(second.operations.len(), 1);
    assert_eq!(second.next_cursor, SyncCursor(3));

    // The page limit is clamped, not trusted.
    let clamped = repository.page_after(SyncCursor(0), 10_000).unwrap();
    assert_eq!(clamped.operations.len(), 3);
    assert!(repository
        .page_after(SyncCursor(3), 10)
        .unwrap()
        .operations
        .is_empty());
}

#[test]
fn revisions_and_server_sequences_are_assigned_by_the_repository() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("revisions.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let mut repository = SqliteSyncRepository::open(&path).unwrap();
    let entity = Uuid::new_v4();
    let other = Uuid::new_v4();

    let created = create(&provider, entity, 1, b"v1");
    assert_eq!(created.base_revision, 0);
    assert_eq!(
        repository.apply_mutation(&created).unwrap(),
        ApplyOutcome::Applied {
            cursor: SyncCursor(1)
        }
    );
    assert_eq!(repository.record(entity).unwrap().unwrap().metadata.revision, 1);

    let updated = update(&provider, entity, 2, 1, b"v2");
    assert_eq!(
        repository.apply_mutation(&updated).unwrap(),
        ApplyOutcome::Applied {
            cursor: SyncCursor(2)
        }
    );
    assert_eq!(repository.record(entity).unwrap().unwrap().metadata.revision, 2);

    // A different entity does not disturb the first one's revision.
    repository
        .apply_mutation(&create(&provider, other, 3, b"other"))
        .unwrap();
    assert_eq!(repository.record(entity).unwrap().unwrap().metadata.revision, 2);
    assert_eq!(repository.record(other).unwrap().unwrap().metadata.revision, 1);

    let page = repository.page_after(SyncCursor(0), 10).unwrap();
    assert_eq!(page.operations[2].entity_revision, 1);
    // Cursor and revision are different quantities.
    assert_ne!(page.operations[2].server_sequence.0, 1);

    // Timestamps never order the journal: the stored one is the client's.
    assert_eq!(updated.timestamp, "2026-01-01T00:00:00Z");
    assert_eq!(page.operations[2].mutation.timestamp, updated.timestamp);
}

#[test]
fn duplicate_device_sequence_is_rejected_without_writing_anything() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("sequence.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let mut repository = SqliteSyncRepository::open(&path).unwrap();

    repository
        .apply_mutation(&create(&provider, Uuid::new_v4(), 1, b"first"))
        .unwrap();
    let collision = create(&provider, Uuid::new_v4(), 1, b"second");
    assert_eq!(
        repository.apply_mutation(&collision).unwrap(),
        ApplyOutcome::Rejected
    );
    // Repeating the refused request stays refused and never consumes a cursor.
    assert_eq!(
        repository.apply_mutation(&collision).unwrap(),
        ApplyOutcome::Rejected
    );
    assert_eq!(
        repository.page_after(SyncCursor(0), 100).unwrap().operations.len(),
        1
    );
}

#[test]
fn base_revision_mismatch_records_a_conflict_and_leaves_the_entity_alone() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("conflict.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let mut repository = SqliteSyncRepository::open(&path).unwrap();
    let entity = Uuid::new_v4();

    repository
        .apply_mutation(&create(&provider, entity, 1, b"desktop version"))
        .unwrap();
    let stale = update(&provider, entity, 2, 0, b"stale android version");
    let outcome = repository.apply_mutation(&stale).unwrap();
    let conflict_id = match outcome {
        ApplyOutcome::Conflict { conflict_id } => conflict_id,
        other => panic!("expected a conflict, got {other:?}"),
    };

    // The entity is untouched.
    assert_eq!(repository.record(entity).unwrap().unwrap().metadata.revision, 1);

    let conflicts: Vec<MutationConflict> = repository.conflicts().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].id, conflict_id);
    assert_eq!(conflicts[0].reason, ConflictReason::BaseRevisionMismatch);
    assert_eq!(conflicts[0].entity_revision, 1);
    assert_eq!(conflicts[0].incoming.operation_id, stale.operation_id);
    let current = conflicts[0].current.as_ref().unwrap();
    assert_eq!(current.metadata.revision, 1);
    assert_eq!(current.metadata.device_id.as_str(), DEVICE);
    let incoming_payload = conflicts[0].incoming.encrypted_payload.clone().unwrap();
    assert_eq!(
        provider
            .decrypt(&PayloadContext::new(ENTITY_TYPE, entity), &incoming_payload)
            .unwrap(),
        b"stale android version"
    );

    // A repeat reports the same conflict without recording a second one, and
    // conflicts are never handed out as replicable changes.
    assert_eq!(
        repository.apply_mutation(&stale).unwrap(),
        ApplyOutcome::Conflict { conflict_id }
    );
    assert_eq!(repository.conflicts().unwrap().len(), 1);
    let page = repository.page_after(SyncCursor(0), 10).unwrap();
    assert_eq!(page.operations.len(), 1);
    // The conflict consumed server sequence 2, so the cursor moved past it.
    assert_eq!(page.next_cursor, SyncCursor(2));

    // The conflict survives a restart with its snapshot intact.
    drop(repository);
    let repository = SqliteSyncRepository::open(&path).unwrap();
    let conflicts = repository.conflicts().unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].id, conflict_id);
    assert_eq!(conflicts[0].current.as_ref().unwrap().metadata.revision, 1);
}

#[test]
fn tombstones_create_a_new_revision_and_block_resurrection() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("tombstone.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let mut repository = SqliteSyncRepository::open(&path).unwrap();
    let entity = Uuid::new_v4();

    repository
        .apply_mutation(&create(&provider, entity, 1, b"to be deleted"))
        .unwrap();
    assert_eq!(
        repository.apply_mutation(&delete(&provider, entity, 2, 1)).unwrap(),
        ApplyOutcome::Applied {
            cursor: SyncCursor(2)
        }
    );
    let record = repository.record(entity).unwrap().unwrap();
    assert!(record.metadata.tombstone);
    assert!(record.content.is_none());
    assert_eq!(record.metadata.revision, 2);

    // An update at the current base revision cannot bring the entity back.
    assert_eq!(
        repository
            .apply_mutation(&update(&provider, entity, 3, 2, b"resurrect"))
            .unwrap(),
        ApplyOutcome::Rejected
    );
    assert_eq!(repository.record(entity).unwrap().unwrap().metadata.revision, 2);
    assert!(repository.record(entity).unwrap().unwrap().metadata.tombstone);

    // A second delete is allowed and produces a new tombstone revision.
    assert_eq!(
        repository.apply_mutation(&delete(&provider, entity, 4, 2)).unwrap(),
        ApplyOutcome::Applied {
            cursor: SyncCursor(3)
        }
    );
    assert_eq!(repository.record(entity).unwrap().unwrap().metadata.revision, 3);
}

#[test]
fn device_cursors_round_trip_and_do_not_move_backwards_on_their_own() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("cursors.sqlite3");
    let mut repository = SqliteSyncRepository::open(&path).unwrap();
    let other = DeviceId::new("second_device").unwrap();

    assert_eq!(repository.device_cursor(&device()).unwrap(), SyncCursor(0));
    repository
        .save_device_cursor(&device(), SyncCursor(7))
        .unwrap();
    repository
        .save_device_cursor(&other, SyncCursor(3))
        .unwrap();
    repository
        .save_device_cursor(&device(), SyncCursor(9))
        .unwrap();

    assert_eq!(repository.device_cursor(&device()).unwrap(), SyncCursor(9));
    assert_eq!(repository.device_cursor(&other).unwrap(), SyncCursor(3));
    assert_eq!(
        repository
            .device_cursor(&DeviceId::new("unknown_device").unwrap())
            .unwrap(),
        SyncCursor(0)
    );

    drop(repository);
    let repository = SqliteSyncRepository::open(&path).unwrap();
    assert_eq!(repository.device_cursor(&device()).unwrap(), SyncCursor(9));
}

#[test]
fn a_failed_transaction_rolls_back_every_partial_write() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("rollback.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let mut repository = SqliteSyncRepository::open(&path).unwrap();
    let entity = Uuid::new_v4();
    repository
        .apply_mutation(&create(&provider, entity, 1, b"committed"))
        .unwrap();

    // Occupy the conflict table's UNIQUE operation_id so the conflict write
    // fails *after* the journal row was already inserted.
    let stale_operation = Uuid::new_v4();
    {
        let raw = Connection::open(&path).unwrap();
        raw.execute(
            "INSERT INTO sync_conflicts(
                conflict_id, entity_id, entity_type, current_revision, current_payload,
                incoming_payload, reason, detected_at, device_id, operation_id,
                base_revision, server_sequence, current_device_id, current_updated_at)
             VALUES(?1, ?2, 'note', 1, NULL, NULL, 'base_revision_mismatch',
                '2026-01-01T00:00:00Z', ?3, ?4, 0, 0, '', '')",
            params![
                Uuid::new_v4().to_string(),
                entity.to_string(),
                DEVICE,
                stale_operation.to_string()
            ],
        )
        .unwrap();
    }

    let stale = mutation(
        &provider,
        stale_operation,
        entity,
        2,
        0,
        SyncOperationKind::Update,
        Some(b"conflicting"),
    );
    assert!(repository.apply_mutation(&stale).is_err());

    // Neither the journal row nor the conflict row was left behind.
    assert_eq!(repository.record(entity).unwrap().unwrap().metadata.revision, 1);
    assert_eq!(
        repository.page_after(SyncCursor(0), 100).unwrap().operations.len(),
        1
    );
    assert!(repository.conflicts().unwrap().is_empty());
    // The reused device sequence was not consumed by the failed attempt.
    assert_eq!(repository.last_device_sequence(&device()).unwrap(), 1);
}

#[test]
fn migrates_an_older_schema_and_keeps_existing_data() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("legacy.sqlite3");
    let entity = Uuid::new_v4();
    let operation = Uuid::new_v4();
    {
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch(SCHEMA_V1_FIXTURE).unwrap();
        raw.execute(
            "INSERT INTO sync_metadata(key, value) VALUES('schema_version', '1')",
            [],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO sync_operations(
                operation_id, entity_id, entity_type, device_id, device_sequence,
                base_revision, entity_revision, operation_kind, tombstone, payload,
                schema_version, updated_at)
             VALUES(?1, ?2, 'note', ?3, 1, 1, 2, 'update', 0, X'0102', 1,
                '2026-01-01T00:00:00Z')",
            params![operation.to_string(), entity.to_string(), DEVICE],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO sync_entities(
                entity_id, entity_type, entity_revision, tombstone, payload,
                last_operation_id, updated_at)
             VALUES(?1, 'note', 2, 0, X'0102', ?2, '2026-01-01T00:00:00Z')",
            params![entity.to_string(), operation.to_string()],
        )
        .unwrap();
        raw.execute("INSERT INTO sync_devices(device_id) VALUES(?1)", [DEVICE])
            .unwrap();
        raw.execute(
            "INSERT INTO sync_device_cursors(device_id, last_server_sequence) VALUES(?1, 1)",
            [DEVICE],
        )
        .unwrap();
    }

    let mut repository = SqliteSyncRepository::open(&path).unwrap();
    assert_eq!(repository.schema_version().unwrap(), SCHEMA_VERSION);
    // Metadata that the older schema did not store was backfilled.
    let record = repository.record(entity).unwrap().unwrap();
    assert_eq!(record.metadata.revision, 2);
    assert_eq!(record.metadata.device_id.as_str(), DEVICE);
    assert_eq!(record.metadata.updated_at, "2026-01-01T00:00:00Z");
    assert!(record.content.is_some());

    let page = repository.page_after(SyncCursor(0), 10).unwrap();
    assert_eq!(page.operations.len(), 1);
    assert_eq!(page.operations[0].outcome, StoredOperationOutcome::Applied);
    assert_eq!(page.operations[0].mutation.operation_id, operation);
    assert_eq!(repository.device_cursor(&device()).unwrap(), SyncCursor(1));

    // Old uniqueness guarantees still hold after the upgrade.
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    assert_eq!(
        repository
            .apply_mutation(&create(&provider, Uuid::new_v4(), 1, b"x"))
            .unwrap(),
        ApplyOutcome::Rejected
    );

    // Reopening an already migrated database is a no-op.
    drop(repository);
    let repository = SqliteSyncRepository::open(&path).unwrap();
    assert_eq!(repository.schema_version().unwrap(), SCHEMA_VERSION);
    assert_eq!(repository.record(entity).unwrap().unwrap().metadata.revision, 2);
}

#[test]
fn a_newer_schema_version_is_refused_instead_of_downgraded() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("newer.sqlite3");
    {
        let raw = Connection::open(&path).unwrap();
        raw.execute_batch(
            "CREATE TABLE sync_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        raw.execute(
            "INSERT INTO sync_metadata(key, value) VALUES('schema_version', '99')",
            [],
        )
        .unwrap();
    }
    assert_eq!(
        SqliteSyncRepository::open(&path).err().unwrap(),
        SyncError::UnsupportedSchemaVersion
    );
}

#[test]
fn a_second_writer_reports_a_controlled_busy_error() {    let directory = tempdir().unwrap();
    let path = directory.path().join("busy.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let mut repository =
        SqliteSyncRepository::open_with_busy_timeout(&path, Duration::from_millis(100)).unwrap();

    let blocker = Connection::open(&path).unwrap();
    blocker
        .execute_batch("PRAGMA journal_mode = WAL; BEGIN IMMEDIATE;")
        .unwrap();

    let outcome = repository.apply_mutation(&create(&provider, Uuid::new_v4(), 1, b"blocked"));
    assert_eq!(outcome, Err(SyncError::StorageBusy));

    blocker.execute_batch("ROLLBACK;").unwrap();
    // Once the lock is gone the very same mutation is applied normally.
    assert_eq!(
        repository
            .apply_mutation(&create(&provider, Uuid::new_v4(), 1, b"blocked"))
            .unwrap(),
        ApplyOutcome::Applied {
            cursor: SyncCursor(1)
        }
    );
}

#[test]
fn protected_payloads_are_never_stored_as_plaintext() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("encrypted.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let entity = Uuid::new_v4();
    let marker = b"FICTIONAL_NOTE_PLAINTEXT_MUST_NOT_PERSIST";
    let mutation = create(&provider, entity, 1, marker);

    {
        let mut repository = SqliteSyncRepository::open(&path).unwrap();
        repository.apply_mutation(&mutation).unwrap();
        repository.checkpoint_and_close().unwrap();
    }

    let stored = database_bytes(&path);
    assert!(!stored.is_empty());
    assert!(
        !contains(&stored, marker),
        "plaintext entity content reached the database file"
    );
    // Diagnostics must not leak it either.
    assert!(!format!("{mutation:?}").contains("FICTIONAL_NOTE_PLAINTEXT"));

    // The ciphertext still round-trips through the stored blob.
    let repository = SqliteSyncRepository::open(&path).unwrap();
    let record = repository.record(entity).unwrap().unwrap();
    let payload = record.content.clone().unwrap();
    assert_eq!(
        provider
            .decrypt(&PayloadContext::new(ENTITY_TYPE, entity), &payload)
            .unwrap(),
        marker
    );
    assert!(!format!("{record:?}").contains("FICTIONAL_NOTE_PLAINTEXT"));
}

#[test]
fn production_storage_lives_outside_the_repository_checkout() {
    let path = SqliteSyncRepository::production_path().unwrap();
    assert!(path.is_absolute(), "{path:?} must be absolute");
    assert_eq!(path.file_name().unwrap(), "sync.sqlite3");
    let working_directory = std::env::current_dir().unwrap();
    assert!(
        !path.starts_with(&working_directory),
        "{path:?} must not live inside the checkout at {working_directory:?}"
    );
}

#[test]
fn invalid_mutations_are_refused_before_any_storage_work() {    let directory = tempdir().unwrap();
    let path = directory.path().join("invalid.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let mut repository = SqliteSyncRepository::open(&path).unwrap();

    let mut zero_sequence = create(&provider, Uuid::new_v4(), 1, b"fixture");
    zero_sequence.device_sequence = 0;
    assert_eq!(
        repository.apply_mutation(&zero_sequence),
        Err(SyncError::InvalidDeviceSequence)
    );

    let mut bad_timestamp = create(&provider, Uuid::new_v4(), 1, b"fixture");
    bad_timestamp.timestamp = "not-a-timestamp".into();
    assert_eq!(
        repository.apply_mutation(&bad_timestamp),
        Err(SyncError::InvalidTimestamp)
    );

    let mut missing_payload = create(&provider, Uuid::new_v4(), 1, b"fixture");
    missing_payload.encrypted_payload = None;
    assert_eq!(
        repository.apply_mutation(&missing_payload),
        Err(SyncError::MissingEncryptedPayload)
    );

    let mut payload_on_delete = delete(&provider, Uuid::new_v4(), 1, 0);
    payload_on_delete.encrypted_payload = create(&provider, Uuid::new_v4(), 1, b"x")
        .encrypted_payload;
    assert_eq!(
        repository.apply_mutation(&payload_on_delete),
        Err(SyncError::InvalidTombstone)
    );

    let mut unsupported_version = create(&provider, Uuid::new_v4(), 1, b"fixture");
    unsupported_version.schema_version = SyncMutation::CURRENT_SCHEMA_VERSION + 1;
    assert_eq!(
        repository.apply_mutation(&unsupported_version),
        Err(SyncError::UnsupportedSchemaVersion)
    );

    assert!(repository
        .page_after(SyncCursor(0), 100)
        .unwrap()
        .operations
        .is_empty());
}

#[test]
fn the_repository_is_usable_through_the_sync_repository_trait() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("trait.sqlite3");
    let provider = MasterKeyCryptoProvider::new(random_master_key().unwrap());
    let entity = Uuid::new_v4();
    let mut repository = SqliteSyncRepository::open(&path).unwrap();

    // This is how the sync engine consumes storage: only through the trait.
    fn submit<R: SyncRepository>(repository: &mut R, mutation: SyncMutation) -> ApplyOutcome {
        repository.apply_mutation(mutation).unwrap()
    }

    assert_eq!(
        submit(&mut repository, create(&provider, entity, 1, b"via the trait")),
        ApplyOutcome::Applied {
            cursor: SyncCursor(1)
        }
    );
    assert_eq!(
        <SqliteSyncRepository as SyncRepository>::record(&repository, entity)
            .unwrap()
            .unwrap()
            .metadata
            .revision,
        1
    );
    assert_eq!(repository.last_device_sequence(&device()).unwrap(), 1);
}
