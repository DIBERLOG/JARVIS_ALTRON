//! Durable SQLite storage for synchronization state.
//!
//! Only identifiers, revisions, cursors, and encrypted payload blobs live here.
//! The repository is the single write authority: one mutation is applied inside
//! one transaction, and a failure never leaves partial data behind.
//!
//! Unencrypted (metadata) columns are: entity ID, entity type, device ID, device
//! sequence, operation ID, operation kind, base revision, entity revision,
//! server sequence, timestamps, tombstone flag, payload format version, and
//! ciphertext length. Payload bytes are always ciphertext produced by the
//! [`super::crypto`] layer.

use super::*;
use rusqlite::{
    named_params, params, Connection, Error as SqlError, ErrorCode, OpenFlags, OptionalExtension,
    Transaction, TransactionBehavior,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

/// Current storage schema. A database that declares a newer version is refused.
pub const SCHEMA_VERSION: i64 = 2;
/// Default lock wait before SQLite reports a busy error.
pub const DEFAULT_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS sync_metadata (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_entities (
    entity_id         TEXT PRIMARY KEY,
    entity_type       TEXT NOT NULL,
    entity_revision   INTEGER NOT NULL,
    tombstone         INTEGER NOT NULL,
    payload           BLOB,
    last_operation_id TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sync_operations (
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
CREATE TABLE IF NOT EXISTS sync_conflicts (
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
CREATE TABLE IF NOT EXISTS sync_devices (
    device_id TEXT PRIMARY KEY
);
CREATE TABLE IF NOT EXISTS sync_device_cursors (
    device_id           TEXT PRIMARY KEY REFERENCES sync_devices(device_id),
    last_server_sequence INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS sync_operations_server_sequence
    ON sync_operations(server_sequence);
";

/// Durable repository.
///
/// Open it with [`SqliteSyncRepository::open`] for an explicit path, or with
/// [`SqliteSyncRepository::open_production`] to use the application data
/// directory. The database never lives inside the repository checkout.
pub struct SqliteSyncRepository {
    connection: Connection,
}

/// Raw `sync_entities` row, before it is validated into a [`SyncRecord`].
struct EntityRow {
    entity_type: String,
    entity_revision: u64,
    tombstone: bool,
    payload: Option<Vec<u8>>,
    device_id: String,
    updated_at: String,
}

/// Raw `sync_operations` row.
struct JournalRow {
    entity_type: String,
    entity_id: String,
    operation_id: String,
    device_id: String,
    device_sequence: u64,
    base_revision: u64,
    entity_revision: u64,
    operation_kind: String,
    payload: Option<Vec<u8>>,
    schema_version: u32,
    updated_at: String,
    outcome: String,
    conflict_id: Option<String>,
    server_sequence: u64,
}

impl SqliteSyncRepository {
    /// Opens (creating when absent) the database at `path`.
    pub fn open(path: &Path) -> Result<Self, SyncError> {
        Self::open_with_busy_timeout(path, DEFAULT_BUSY_TIMEOUT)
    }

    /// Opens the database with an explicit lock wait, mainly for diagnostics.
    pub fn open_with_busy_timeout(
        path: &Path,
        busy_timeout: Duration,
    ) -> Result<Self, SyncError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(storage_error)?;
        connection
            .busy_timeout(busy_timeout)
            .map_err(storage_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(storage_error)?;
        let foreign_keys: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .map_err(storage_error)?;
        if foreign_keys != 1 {
            // Referential integrity is part of the storage contract.
            return Err(SyncError::StorageUnavailable);
        }
        connection
            .execute_batch("PRAGMA synchronous = FULL;")
            .map_err(storage_error)?;
        // WAL is not assumed: the pragma must report the mode it actually took.
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
            .map_err(storage_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(SyncError::StorageUnavailable);
        }

        let mut repository = Self { connection };
        repository.migrate()?;
        Ok(repository)
    }

    /// Production location: the per-user application data directory.
    pub fn production_path() -> Result<PathBuf, SyncError> {
        let dirs = platform_dirs::AppDirs::new(Some(crate::config::BUNDLE_IDENTIFIER), false)
            .ok_or(SyncError::StorageUnavailable)?;
        Ok(dirs.data_dir.join("sync.sqlite3"))
    }

    /// Opens the production database, creating its directory when needed.
    pub fn open_production() -> Result<Self, SyncError> {
        let path = Self::production_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| SyncError::StorageUnavailable)?;
        }
        Self::open(&path)
    }

    /// Opens `app_data_dir/sync.sqlite3`, creating the directory when needed.
    pub fn open_at_app_data(app_data_dir: &Path) -> Result<Self, SyncError> {
        std::fs::create_dir_all(app_data_dir).map_err(|_| SyncError::StorageUnavailable)?;
        Self::open(&app_data_dir.join("sync.sqlite3"))
    }

    /// Schema version currently recorded in the database.
    pub fn schema_version(&self) -> Result<i64, SyncError> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM sync_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage_error)?;
        match value {
            Some(text) => text.parse::<i64>().map_err(|_| SyncError::StorageCorrupt),
            None => Ok(0),
        }
    }

    /// Applies one mutation: all checks and writes share a single transaction.
    ///
    /// Order of work: operation-ID check, device-sequence check, current entity
    /// read, base-revision check, revision and server-sequence assignment,
    /// journal write, then the entity or conflict write, then commit.
    pub fn apply_mutation(&mut self, mutation: &SyncMutation) -> Result<ApplyOutcome, SyncError> {
        validate_mutation(mutation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let outcome = apply_in_transaction(&transaction, mutation)?;
        transaction.commit().map_err(storage_error)?;
        Ok(outcome)
    }

    /// Current entity state.
    pub fn record(&self, entity_id: Uuid) -> Result<Option<SyncRecord>, SyncError> {
        read_entity(&self.connection, entity_id)
    }

    /// Applied journal entries after `cursor`, at most `limit` per page.
    ///
    /// Conflicts consume a server sequence and advance `next_cursor`, but they
    /// are never returned as replicable changes.
    pub fn page_after(&self, cursor: SyncCursor, limit: usize) -> Result<SyncPage, SyncError> {
        let limit = limit.min(MAX_PAGE_SIZE);
        if limit == 0 {
            return Ok(SyncPage {
                operations: Vec::new(),
                next_cursor: cursor,
            });
        }
        let mut after = cursor.0;
        let mut operations = Vec::with_capacity(limit);
        loop {
            let remaining = limit - operations.len();
            let batch = journal_batch(&self.connection, after, remaining)?;
            if batch.is_empty() {
                break;
            }
            let batch_len = batch.len();
            after = batch[batch_len - 1].server_sequence;
            for row in batch {
                if row.outcome == OUTCOME_APPLIED {
                    operations.push(stored_operation(row)?);
                }
            }
            if operations.len() >= limit || batch_len < remaining {
                break;
            }
        }
        Ok(SyncPage {
            operations,
            next_cursor: SyncCursor(after),
        })
    }

    /// Retained conflicts, ordered by the journal position where they occurred.
    pub fn conflicts(&self) -> Result<Vec<MutationConflict>, SyncError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT c.conflict_id, c.entity_id, c.reason, c.current_revision,
                        c.current_payload, c.current_device_id, c.current_updated_at,
                        c.entity_type, o.operation_id, o.device_id, o.device_sequence,
                        o.base_revision, o.operation_kind, o.schema_version, o.payload,
                        o.entity_revision, o.server_sequence, o.updated_at
                 FROM sync_conflicts c
                 JOIN sync_operations o ON o.operation_id = c.operation_id
                 ORDER BY o.server_sequence",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u64>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, u64>(10)?,
                    row.get::<_, u64>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, u32>(13)?,
                    row.get::<_, Option<Vec<u8>>>(14)?,
                    row.get::<_, u64>(15)?,
                    row.get::<_, u64>(16)?,
                    row.get::<_, String>(17)?,
                ))
            })
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error)?;

        rows.into_iter()
            .map(
                |(
                    conflict_id,
                    entity_id,
                    reason,
                    current_revision,
                    current_payload,
                    current_device_id,
                    current_updated_at,
                    entity_type,
                    operation_id,
                    device_id,
                    device_sequence,
                    base_revision,
                    operation_kind,
                    schema_version,
                    payload,
                    entity_revision,
                    server_sequence,
                    updated_at,
                )| {
                    let entity_type = SyncEntityType::from_storage_name(&entity_type)?;
                    let entity_id = parse_uuid(&entity_id)?;
                    let current = if current_revision == 0 && current_payload.is_none() {
                        None
                    } else {
                        Some(SyncRecord {
                            metadata: SyncRecordMetadata {
                                id: entity_id,
                                entity_type: entity_type.clone(),
                                revision: current_revision,
                                device_id: DeviceId::new(current_device_id)?,
                                updated_at: current_updated_at,
                                tombstone: current_payload.is_none(),
                            },
                            content: current_payload.map(EncryptedPayload::from_opaque_bytes),
                        })
                    };
                    Ok(MutationConflict {
                        id: parse_uuid(&conflict_id)?,
                        entity_id,
                        entity_type: entity_type.clone(),
                        reason: ConflictReason::from_storage_name(&reason)?,
                        current,
                        incoming: SyncMutation {
                            operation_id: parse_uuid(&operation_id)?,
                            entity_id,
                            entity_type,
                            device_id: DeviceId::new(device_id)?,
                            device_sequence,
                            base_revision,
                            kind: SyncOperationKind::from_storage_name(&operation_kind)?,
                            timestamp: updated_at,
                            schema_version,
                            encrypted_payload: payload.map(EncryptedPayload::from_opaque_bytes),
                        },
                        entity_revision,
                        server_sequence: SyncCursor(server_sequence),
                    })
                },
            )
            .collect()
    }

    /// Highest server sequence already pulled by `device`.
    pub fn device_cursor(&self, device: &DeviceId) -> Result<SyncCursor, SyncError> {
        let value: Option<u64> = self
            .connection
            .query_row(
                "SELECT last_server_sequence FROM sync_device_cursors WHERE device_id = ?1",
                [device.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage_error)?;
        Ok(SyncCursor(value.unwrap_or(0)))
    }

    /// Stores a device's pull position.
    pub fn save_device_cursor(
        &mut self,
        device: &DeviceId,
        cursor: SyncCursor,
    ) -> Result<(), SyncError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        transaction
            .execute(
                "INSERT INTO sync_devices(device_id) VALUES(?1)
                 ON CONFLICT(device_id) DO NOTHING",
                [device.as_str()],
            )
            .map_err(storage_error)?;
        transaction
            .execute(
                "INSERT INTO sync_device_cursors(device_id, last_server_sequence)
                 VALUES(?1, ?2)
                 ON CONFLICT(device_id) DO UPDATE
                 SET last_server_sequence = excluded.last_server_sequence",
                params![device.as_str(), cursor.0],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)
    }

    /// Highest device sequence already used by `device`, or 0.
    pub fn last_device_sequence(&self, device: &DeviceId) -> Result<u64, SyncError> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(device_sequence), 0) FROM sync_operations
                 WHERE device_id = ?1",
                [device.as_str()],
                |row| row.get(0),
            )
            .map_err(storage_error)
    }

    /// Forgets a retained conflict after an operator resolved it.
    ///
    /// The journal row is kept, so the incoming version stays auditable.
    pub fn discard_conflict(&mut self, conflict_id: Uuid) -> Result<bool, SyncError> {
        let removed = self
            .connection
            .execute(
                "DELETE FROM sync_conflicts WHERE conflict_id = ?1",
                [conflict_id.to_string()],
            )
            .map_err(storage_error)?;
        Ok(removed > 0)
    }

    /// Flushes the write-ahead log and closes the connection cleanly.
    pub fn checkpoint_and_close(self) -> Result<(), SyncError> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(storage_error)?;
        self.connection.close().map_err(|_| SyncError::StorageUnavailable)
    }

    fn migrate(&mut self) -> Result<(), SyncError> {
        // DDL in SQLite is transactional, so a failed migration can be retried
        // from the version it started at.
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let current = schema_version(&transaction)?;
        if current > SCHEMA_VERSION {
            return Err(SyncError::UnsupportedSchemaVersion);
        }
        if current < 0 {
            return Err(SyncError::StorageCorrupt);
        }
        if current < 1 {
            transaction.execute_batch(SCHEMA_V1).map_err(storage_error)?;
        }
        if current < 2 {
            migrate_to_v2(&transaction)?;
        }
        transaction
            .execute(
                "INSERT INTO sync_metadata(key, value) VALUES('schema_version', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [SCHEMA_VERSION.to_string()],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)
    }
}

impl SyncRepository for SqliteSyncRepository {
    fn apply_mutation(&mut self, mutation: SyncMutation) -> Result<ApplyOutcome, SyncError> {
        // Delegates to the inherent method: same transaction, borrowed request.
        SqliteSyncRepository::apply_mutation(self, &mutation)
    }

    fn record(&self, entity_id: Uuid) -> Result<Option<SyncRecord>, SyncError> {
        SqliteSyncRepository::record(self, entity_id)
    }

    fn page_after(&self, cursor: SyncCursor, limit: usize) -> Result<SyncPage, SyncError> {
        SqliteSyncRepository::page_after(self, cursor, limit)
    }

    fn conflicts(&self) -> Result<Vec<MutationConflict>, SyncError> {
        SqliteSyncRepository::conflicts(self)
    }

    fn device_cursor(&self, device: &DeviceId) -> Result<SyncCursor, SyncError> {
        SqliteSyncRepository::device_cursor(self, device)
    }

    fn save_device_cursor(
        &mut self,
        device: &DeviceId,
        cursor: SyncCursor,
    ) -> Result<(), SyncError> {
        SqliteSyncRepository::save_device_cursor(self, device, cursor)
    }

    fn last_device_sequence(&self, device: &DeviceId) -> Result<u64, SyncError> {
        SqliteSyncRepository::last_device_sequence(self, device)
    }

    fn discard_conflict(&mut self, conflict_id: Uuid) -> Result<bool, SyncError> {
        SqliteSyncRepository::discard_conflict(self, conflict_id)
    }
}

const OUTCOME_APPLIED: &str = "applied";
const OUTCOME_CONFLICT: &str = "conflict";

/// The whole mutation protocol, inside one caller-provided transaction.
fn apply_in_transaction(
    transaction: &Transaction<'_>,
    mutation: &SyncMutation,
) -> Result<ApplyOutcome, SyncError> {
    // 1. An operation ID is applied at most once; a repeat changes nothing.
    let existing: Option<(u64, String, Option<String>)> = transaction
        .query_row(
            "SELECT server_sequence, outcome, conflict_id FROM sync_operations
             WHERE operation_id = ?1",
            [mutation.operation_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(storage_error)?;
    if let Some((server_sequence, outcome, conflict_id)) = existing {
        return Ok(if outcome == OUTCOME_CONFLICT {
            ApplyOutcome::Conflict {
                conflict_id: parse_uuid(conflict_id.as_deref().unwrap_or_default())?,
            }
        } else {
            ApplyOutcome::AlreadyApplied {
                cursor: SyncCursor(server_sequence),
            }
        });
    }

    // 2. A device sequence may be spent once by this device.
    let sequence_taken: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM sync_operations WHERE device_id = ?1 AND device_sequence = ?2",
            params![mutation.device_id.as_str(), mutation.device_sequence],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage_error)?;
    if sequence_taken.is_some() {
        // Nothing was written, so the transaction rolls back on drop.
        return Ok(ApplyOutcome::Rejected);
    }

    // 3. Read the entity that the mutation claims to modify.
    let current = read_entity_row(transaction, mutation.entity_id)?;
    let current_revision = current
        .as_ref()
        .map(|row| row.entity_revision)
        .unwrap_or(0);

    // 4. The base revision decides whether the entity may be touched at all.
    if current_revision != mutation.base_revision {
        let conflict_id = Uuid::new_v4();
        let server_sequence = insert_operation(
            transaction,
            mutation,
            current_revision,
            OUTCOME_CONFLICT,
            Some(conflict_id),
        )?;
        transaction
            .execute(
                "INSERT INTO sync_conflicts(
                    conflict_id, entity_id, entity_type, current_revision, current_payload,
                    incoming_payload, reason, detected_at, device_id, operation_id,
                    base_revision, server_sequence, current_device_id, current_updated_at)
                 VALUES(:conflict_id, :entity_id, :entity_type, :current_revision,
                    :current_payload, :incoming_payload, :reason, :detected_at, :device_id,
                    :operation_id, :base_revision, :server_sequence, :current_device_id,
                    :current_updated_at)",
                named_params! {
                    ":conflict_id": conflict_id.to_string(),
                    ":entity_id": mutation.entity_id.to_string(),
                    ":entity_type": mutation.entity_type.as_str(),
                    ":current_revision": current_revision,
                    ":current_payload": current.as_ref().and_then(|row| row.payload.clone()),
                    ":incoming_payload": mutation
                        .encrypted_payload
                        .as_ref()
                        .map(EncryptedPayload::as_opaque_bytes),
                    ":reason": ConflictReason::BaseRevisionMismatch.as_str(),
                    ":detected_at": mutation.timestamp,
                    ":device_id": mutation.device_id.as_str(),
                    ":operation_id": mutation.operation_id.to_string(),
                    ":base_revision": mutation.base_revision,
                    ":server_sequence": server_sequence,
                    ":current_device_id": current
                        .as_ref()
                        .map(|row| row.device_id.as_str())
                        .unwrap_or_default(),
                    ":current_updated_at": current
                        .as_ref()
                        .map(|row| row.updated_at.as_str())
                        .unwrap_or_default(),
                },
            )
            .map_err(storage_error)?;
        return Ok(ApplyOutcome::Conflict { conflict_id });
    }

    // 5. A tombstone is only replaceable by another delete at the next revision.
    if current.as_ref().is_some_and(|row| row.tombstone)
        && !matches!(mutation.kind, SyncOperationKind::Delete)
    {
        return Ok(ApplyOutcome::Rejected);
    }

    let entity_revision = current_revision + 1;
    let server_sequence = insert_operation(
        transaction,
        mutation,
        entity_revision,
        OUTCOME_APPLIED,
        None,
    )?;
    let tombstone = matches!(mutation.kind, SyncOperationKind::Delete);
    transaction
        .execute(
            "INSERT INTO sync_entities(
                entity_id, entity_type, entity_revision, tombstone, payload,
                last_operation_id, updated_at, device_id)
             VALUES(:entity_id, :entity_type, :entity_revision, :tombstone, :payload,
                :last_operation_id, :updated_at, :device_id)
             ON CONFLICT(entity_id) DO UPDATE SET
                entity_type = excluded.entity_type,
                entity_revision = excluded.entity_revision,
                tombstone = excluded.tombstone,
                payload = excluded.payload,
                last_operation_id = excluded.last_operation_id,
                updated_at = excluded.updated_at,
                device_id = excluded.device_id",
            named_params! {
                ":entity_id": mutation.entity_id.to_string(),
                ":entity_type": mutation.entity_type.as_str(),
                ":entity_revision": entity_revision,
                ":tombstone": tombstone,
                ":payload": if tombstone {
                    None
                } else {
                    mutation
                        .encrypted_payload
                        .as_ref()
                        .map(EncryptedPayload::as_opaque_bytes)
                },
                ":last_operation_id": mutation.operation_id.to_string(),
                ":updated_at": mutation.timestamp,
                ":device_id": mutation.device_id.as_str(),
            },
        )
        .map_err(storage_error)?;
    Ok(ApplyOutcome::Applied {
        cursor: SyncCursor(server_sequence),
    })
}

/// Appends the journal row and returns the assigned server sequence.
fn insert_operation(
    transaction: &Transaction<'_>,
    mutation: &SyncMutation,
    entity_revision: u64,
    outcome: &str,
    conflict_id: Option<Uuid>,
) -> Result<u64, SyncError> {
    transaction
        .execute(
            "INSERT INTO sync_operations(
                operation_id, entity_id, entity_type, device_id, device_sequence,
                base_revision, entity_revision, operation_kind, tombstone, payload,
                schema_version, updated_at, outcome, conflict_id)
             VALUES(:operation_id, :entity_id, :entity_type, :device_id, :device_sequence,
                :base_revision, :entity_revision, :operation_kind, :tombstone, :payload,
                :schema_version, :updated_at, :outcome, :conflict_id)",
            named_params! {
                ":operation_id": mutation.operation_id.to_string(),
                ":entity_id": mutation.entity_id.to_string(),
                ":entity_type": mutation.entity_type.as_str(),
                ":device_id": mutation.device_id.as_str(),
                ":device_sequence": mutation.device_sequence,
                ":base_revision": mutation.base_revision,
                ":entity_revision": entity_revision,
                ":operation_kind": mutation.kind.as_str(),
                ":tombstone": matches!(mutation.kind, SyncOperationKind::Delete),
                ":payload": mutation
                    .encrypted_payload
                    .as_ref()
                    .map(EncryptedPayload::as_opaque_bytes),
                ":schema_version": mutation.schema_version,
                ":updated_at": mutation.timestamp,
                ":outcome": outcome,
                ":conflict_id": conflict_id.map(|id| id.to_string()),
            },
        )
        .map_err(storage_error)?;
    Ok(transaction.last_insert_rowid() as u64)
}

fn read_entity(connection: &Connection, entity_id: Uuid) -> Result<Option<SyncRecord>, SyncError> {
    let row = read_entity_row(connection, entity_id)?;
    match row {
        None => Ok(None),
        Some(row) => Ok(Some(SyncRecord {
            metadata: SyncRecordMetadata {
                id: entity_id,
                entity_type: SyncEntityType::from_storage_name(&row.entity_type)?,
                revision: row.entity_revision,
                device_id: DeviceId::new(row.device_id)?,
                updated_at: row.updated_at,
                tombstone: row.tombstone,
            },
            content: row.payload.map(EncryptedPayload::from_opaque_bytes),
        })),
    }
}

#[allow(clippy::type_complexity)]
fn read_entity_row(
    connection: &Connection,
    entity_id: Uuid,
) -> Result<Option<EntityRow>, SyncError> {
    let row: Option<(String, u64, bool, Option<Vec<u8>>, String, String)> = connection
        .query_row(
            "SELECT entity_type, entity_revision, tombstone, payload, device_id, updated_at
             FROM sync_entities WHERE entity_id = ?1",
            [entity_id.to_string()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?;
    let row = match row {
        None => return Ok(None),
        Some((
            entity_type,
            entity_revision,
            tombstone,
            payload,
            device_id,
            updated_at,
        )) => EntityRow {
            entity_type,
            entity_revision,
            tombstone,
            payload,
            device_id,
            updated_at,
        },
    };
    if row.entity_revision == 0 {
        return Err(SyncError::StorageCorrupt);
    }
    Ok(Some(row))
}

#[allow(clippy::type_complexity)]
fn journal_batch(
    connection: &Connection,
    after: u64,
    limit: usize,
) -> Result<Vec<JournalRow>, SyncError> {
    let mut statement = connection
        .prepare(
            "SELECT entity_type, entity_id, operation_id, device_id, device_sequence,
                    base_revision, entity_revision, operation_kind, payload,
                    schema_version, updated_at, outcome, conflict_id, server_sequence
             FROM sync_operations
             WHERE server_sequence > ?1
             ORDER BY server_sequence
             LIMIT ?2",
        )
        .map_err(storage_error)?;
    let rows = statement
        .query_map(params![after, limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, u64>(5)?,
                row.get::<_, u64>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<Vec<u8>>>(8)?,
                row.get::<_, u32>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, u64>(13)?,
            ))
        })
        .map_err(storage_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(storage_error)?;

    rows.into_iter()
        .map(
            |(
                entity_type,
                entity_id,
                operation_id,
                device_id,
                device_sequence,
                base_revision,
                entity_revision,
                operation_kind,
                payload,
                schema_version,
                updated_at,
                outcome,
                conflict_id,
                server_sequence,
            )| {
                Ok(JournalRow {
                    entity_type,
                    entity_id,
                    operation_id,
                    device_id,
                    device_sequence,
                    base_revision,
                    entity_revision,
                    operation_kind,
                    payload,
                    schema_version,
                    updated_at,
                    outcome,
                    conflict_id,
                    server_sequence,
                })
            },
        )
        .collect()
}

fn stored_operation(row: JournalRow) -> Result<StoredSyncOperation, SyncError> {
    let entity_type = SyncEntityType::from_storage_name(&row.entity_type)?;
    let conflict_id = match row.conflict_id.as_deref() {
        Some(value) => Some(parse_uuid(value)?),
        None => None,
    };
    Ok(StoredSyncOperation {
        mutation: SyncMutation {
            operation_id: parse_uuid(&row.operation_id)?,
            entity_id: parse_uuid(&row.entity_id)?,
            entity_type,
            device_id: DeviceId::new(row.device_id)?,
            device_sequence: row.device_sequence,
            base_revision: row.base_revision,
            kind: SyncOperationKind::from_storage_name(&row.operation_kind)?,
            timestamp: row.updated_at,
            schema_version: row.schema_version,
            encrypted_payload: row.payload.map(EncryptedPayload::from_opaque_bytes),
        },
        entity_revision: row.entity_revision,
        server_sequence: SyncCursor(row.server_sequence),
        outcome: if row.outcome == OUTCOME_CONFLICT {
            StoredOperationOutcome::Conflict
        } else {
            StoredOperationOutcome::Applied
        },
        conflict_id,
    })
}

fn parse_uuid(value: &str) -> Result<Uuid, SyncError> {
    Uuid::parse_str(value).map_err(|_| SyncError::StorageCorrupt)
}

/// Schema step 1 -> 2: entity device IDs and journal/conflict outcome columns.
fn migrate_to_v2(transaction: &Transaction<'_>) -> Result<(), SyncError> {
    add_column_if_missing(
        transaction,
        "sync_entities",
        "device_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    // Backfill what the older schema did not record.
    transaction
        .execute_batch(
            "UPDATE sync_entities SET device_id = COALESCE(
                 (SELECT o.device_id FROM sync_operations o
                  WHERE o.operation_id = sync_entities.last_operation_id), '')
             WHERE device_id = '';",
        )
        .map_err(storage_error)?;
    add_column_if_missing(
        transaction,
        "sync_operations",
        "outcome",
        "TEXT NOT NULL DEFAULT 'applied'",
    )?;
    add_column_if_missing(transaction, "sync_operations", "conflict_id", "TEXT")?;
    add_column_if_missing(
        transaction,
        "sync_conflicts",
        "base_revision",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        transaction,
        "sync_conflicts",
        "server_sequence",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        transaction,
        "sync_conflicts",
        "current_device_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        transaction,
        "sync_conflicts",
        "current_updated_at",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    transaction
        .execute_batch(
            "UPDATE sync_conflicts SET server_sequence = COALESCE(
                 (SELECT o.server_sequence FROM sync_operations o
                  WHERE o.operation_id = sync_conflicts.operation_id), 0)
             WHERE server_sequence = 0;
             UPDATE sync_conflicts SET base_revision = COALESCE(
                 (SELECT o.base_revision FROM sync_operations o
                  WHERE o.operation_id = sync_conflicts.operation_id), 0)
             WHERE base_revision = 0;
             UPDATE sync_conflicts SET current_device_id = COALESCE(
                 (SELECT e.device_id FROM sync_entities e
                  WHERE e.entity_id = sync_conflicts.entity_id), '')
             WHERE current_device_id = '';
             UPDATE sync_conflicts SET current_updated_at = COALESCE(
                 (SELECT e.updated_at FROM sync_entities e
                  WHERE e.entity_id = sync_conflicts.entity_id), '')
             WHERE current_updated_at = '';
             CREATE INDEX IF NOT EXISTS sync_conflicts_entity
                 ON sync_conflicts(entity_id);",
        )
        .map_err(storage_error)
}

/// `ALTER TABLE ... ADD COLUMN` has no `IF NOT EXISTS`, so check first.
fn add_column_if_missing(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), SyncError> {
    if column_exists(transaction, table, column)? {
        return Ok(());
    }
    transaction
        .execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition};"
        ))
        .map_err(storage_error)
}

fn column_exists(
    transaction: &Transaction<'_>,
    table: &str,
    column: &str,
) -> Result<bool, SyncError> {
    let mut statement = transaction
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(storage_error)?;
    let mut rows = statement.query([]).map_err(storage_error)?;
    while let Some(row) = rows.next().map_err(storage_error)? {
        let name: String = row.get(1).map_err(storage_error)?;
        if name == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn schema_version(transaction: &Transaction<'_>) -> Result<i64, SyncError> {
    let has_metadata: Option<i64> = transaction
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'sync_metadata'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage_error)?;
    if has_metadata.is_none() {
        return Ok(0);
    }
    let value: Option<String> = transaction
        .query_row(
            "SELECT value FROM sync_metadata WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(storage_error)?;
    match value {
        None => Ok(0),
        Some(text) => text.parse::<i64>().map_err(|_| SyncError::StorageCorrupt),
    }
}

/// Maps SQLite failures onto controlled errors without exposing stored data.
fn storage_error(error: SqlError) -> SyncError {
    match &error {
        SqlError::SqliteFailure(code, _) => match code.code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => SyncError::StorageBusy,
            ErrorCode::ConstraintViolation => SyncError::StorageCorrupt,
            _ => SyncError::StorageUnavailable,
        },
        SqlError::InvalidColumnType(..)
        | SqlError::InvalidColumnIndex(..)
        | SqlError::FromSqlConversionFailure(..)
        | SqlError::IntegralValueOutOfRange(..) => SyncError::StorageCorrupt,
        _ => SyncError::StorageUnavailable,
    }
}
