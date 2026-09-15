//! Durable SQLite storage for opaque synchronization records.
//!
//! Only identifiers, revisions, cursors and encrypted payload blobs are stored.

use super::*;
use rusqlite::{params, Connection, Error as SqlError, OpenFlags, OptionalExtension};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const SCHEMA_VERSION: i64 = 2;

pub struct SqliteSyncRepository {
    connection: Connection,
}

impl SqliteSyncRepository {
    pub fn open(path: &Path) -> Result<Self, SyncError> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(storage_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(storage_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = FULL;")
            .map_err(storage_error)?;
        let journal_mode: String = connection
            .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
            .map_err(storage_error)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(SyncError::StorageUnavailable);
        }
        let mut repository = Self { connection };
        repository.migrate()?;
        Ok(repository)
    }
    pub fn default_path(app_data_dir: &Path) -> PathBuf {
        app_data_dir.join("jarvis").join("sync.sqlite3")
    }
    pub fn open_at_app_data(app_data_dir: &Path) -> Result<Self, SyncError> {
        let path = Self::default_path(app_data_dir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| SyncError::StorageUnavailable)?;
        }
        Self::open(&path)
    }
    fn migrate(&mut self) -> Result<(), SyncError> {
        let tx = self.connection.transaction().map_err(storage_error)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS sync_metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL); CREATE TABLE IF NOT EXISTS sync_entities (entity_id TEXT PRIMARY KEY, entity_type TEXT NOT NULL, entity_revision INTEGER NOT NULL, tombstone INTEGER NOT NULL, payload BLOB, last_operation_id TEXT NOT NULL, updated_at TEXT NOT NULL); CREATE TABLE IF NOT EXISTS sync_operations (server_sequence INTEGER PRIMARY KEY AUTOINCREMENT, operation_id TEXT NOT NULL UNIQUE, entity_id TEXT NOT NULL, entity_type TEXT NOT NULL, device_id TEXT NOT NULL, device_sequence INTEGER NOT NULL, base_revision INTEGER NOT NULL, entity_revision INTEGER NOT NULL, operation_kind TEXT NOT NULL, tombstone INTEGER NOT NULL, payload BLOB, schema_version INTEGER NOT NULL, updated_at TEXT NOT NULL, outcome TEXT NOT NULL DEFAULT 'applied', conflict_id TEXT, UNIQUE(device_id, device_sequence)); CREATE INDEX IF NOT EXISTS sync_operations_server_sequence ON sync_operations(server_sequence); CREATE TABLE IF NOT EXISTS sync_conflicts (conflict_id TEXT PRIMARY KEY, entity_id TEXT NOT NULL, entity_type TEXT NOT NULL, current_revision INTEGER NOT NULL, current_payload BLOB, incoming_payload BLOB, reason TEXT NOT NULL, detected_at TEXT NOT NULL, device_id TEXT NOT NULL, operation_id TEXT NOT NULL UNIQUE); CREATE TABLE IF NOT EXISTS sync_devices (device_id TEXT PRIMARY KEY); CREATE TABLE IF NOT EXISTS sync_device_cursors (device_id TEXT PRIMARY KEY REFERENCES sync_devices(device_id), last_server_sequence INTEGER NOT NULL DEFAULT 0);").map_err(storage_error)?;
        let version: Option<i64> = tx
            .query_row(
                "SELECT value FROM sync_metadata WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(0).map(|v| v.parse().unwrap_or(-1)),
            )
            .optional()
            .map_err(storage_error)?;
        if version.is_some_and(|v| !(1..=SCHEMA_VERSION).contains(&v)) {
            return Err(SyncError::UnsupportedSchemaVersion);
        }
        if version == Some(1) {
            tx.execute_batch("ALTER TABLE sync_operations ADD COLUMN outcome TEXT NOT NULL DEFAULT 'applied'; ALTER TABLE sync_operations ADD COLUMN conflict_id TEXT;").map_err(storage_error)?;
        }
        tx.execute("INSERT INTO sync_metadata(key,value) VALUES('schema_version',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [SCHEMA_VERSION.to_string()]).map_err(storage_error)?;
        tx.commit().map_err(storage_error)
    }
    pub fn schema_version(&self) -> Result<i64, SyncError> {
        self.connection
            .query_row(
                "SELECT value FROM sync_metadata WHERE key='schema_version'",
                [],
                |r| r.get::<_, String>(0).map(|v| v.parse().unwrap_or(-1)),
            )
            .map_err(storage_error)
    }
    pub fn record(&self, entity_id: Uuid) -> Result<Option<SyncRecord>, SyncError> {
        self.connection.query_row("SELECT entity_type,entity_revision,tombstone,payload,updated_at FROM sync_entities WHERE entity_id=?1", [entity_id.to_string()], |r| Ok(SyncRecord { metadata: SyncRecordMetadata { id:entity_id, entity_type:parse_entity(&r.get::<_,String>(0)?)?, revision:r.get(1)?, device_id:DeviceId::new("stored").map_err(|_|rusqlite::Error::InvalidQuery)?, updated_at:r.get(4)?, tombstone:r.get(2)? }, content:r.get::<_,Option<Vec<u8>>>(3)?.map(EncryptedPayload::from_opaque_bytes) })).optional().map_err(storage_error)
    }
    pub fn changes_after(
        &self,
        cursor: SyncCursor,
        limit: usize,
    ) -> Result<Vec<StoredSyncOperation>, SyncError> {
        let mut s=self.connection.prepare("SELECT server_sequence,operation_id,entity_id,entity_type,device_id,device_sequence,base_revision,entity_revision,operation_kind,payload,schema_version,updated_at,outcome FROM sync_operations WHERE server_sequence>?1 ORDER BY server_sequence LIMIT ?2").map_err(storage_error)?;
        let result = s
            .query_map(params![cursor.0, limit.min(MAX_PAGE_SIZE)], row_to_stored)
            .map_err(storage_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage_error);
        result
    }
    pub fn save_device_cursor(
        &mut self,
        device: &DeviceId,
        cursor: SyncCursor,
    ) -> Result<(), SyncError> {
        self.connection
            .execute(
                "INSERT INTO sync_devices(device_id) VALUES(?1) ON CONFLICT(device_id) DO NOTHING",
                [device.as_str()],
            )
            .map_err(storage_error)?;
        self.connection.execute("INSERT INTO sync_device_cursors(device_id,last_server_sequence) VALUES(?1,?2) ON CONFLICT(device_id) DO UPDATE SET last_server_sequence=excluded.last_server_sequence",params![device.as_str(),cursor.0]).map_err(storage_error)?;
        Ok(())
    }
    pub fn device_cursor(&self, device: &DeviceId) -> Result<SyncCursor, SyncError> {
        Ok(SyncCursor(
            self.connection
                .query_row(
                    "SELECT last_server_sequence FROM sync_device_cursors WHERE device_id=?1",
                    [device.as_str()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage_error)?
                .unwrap_or(0),
        ))
    }
    pub fn apply_mutation(&mut self, m: &SyncMutation) -> Result<ApplyOutcome, SyncError> {
        validate_mutation(m)?;
        let tx = self.connection.transaction().map_err(storage_error)?;
        if let Some((seq,outcome,id))=tx.query_row("SELECT server_sequence,outcome,conflict_id FROM sync_operations WHERE operation_id=?1",[m.operation_id.to_string()],|r|Ok((r.get::<_,u64>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).optional().map_err(storage_error)? {return Ok(if outcome=="conflict" {ApplyOutcome::Conflict{conflict_id:parse_uuid(id.as_deref().unwrap_or(""))?}} else {ApplyOutcome::AlreadyApplied{cursor:seq}})}
        if tx
            .query_row(
                "SELECT 1 FROM sync_operations WHERE device_id=?1 AND device_sequence=?2",
                params![m.device_id.as_str(), m.device_sequence],
                |_| Ok(()),
            )
            .optional()
            .map_err(storage_error)?
            .is_some()
        {
            return Ok(ApplyOutcome::Rejected);
        }
        let current: Option<(u64, bool, Option<Vec<u8>>)> = tx
            .query_row(
                "SELECT entity_revision,tombstone,payload FROM sync_entities WHERE entity_id=?1",
                [m.entity_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(storage_error)?;
        let revision = current.as_ref().map(|v| v.0).unwrap_or(0);
        let conflict = revision != m.base_revision;
        let tombstoned =
            current.as_ref().is_some_and(|v| v.1) && !matches!(m.kind, SyncOperationKind::Delete);
        let id = conflict.then(Uuid::new_v4);
        if tombstoned {
            return Ok(ApplyOutcome::Rejected);
        }
        tx.execute("INSERT INTO sync_operations(operation_id,entity_id,entity_type,device_id,device_sequence,base_revision,entity_revision,operation_kind,tombstone,payload,schema_version,updated_at,outcome,conflict_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![m.operation_id.to_string(),m.entity_id.to_string(),entity_name(&m.entity_type),m.device_id.as_str(),m.device_sequence,m.base_revision,if conflict{revision}else{revision+1},kind_name(&m.kind),matches!(m.kind,SyncOperationKind::Delete),m.encrypted_payload.as_ref().map(EncryptedPayload::as_opaque_bytes),m.schema_version,m.timestamp,if conflict{"conflict"}else{"applied"},id.map(|v|v.to_string())]).map_err(storage_error)?;
        let sequence = tx.last_insert_rowid() as u64;
        if let Some(id) = id {
            tx.execute("INSERT INTO sync_conflicts(conflict_id,entity_id,entity_type,current_revision,current_payload,incoming_payload,reason,detected_at,device_id,operation_id) VALUES(?1,?2,?3,?4,?5,?6,'base_revision_mismatch',?7,?8,?9)",params![id.to_string(),m.entity_id.to_string(),entity_name(&m.entity_type),revision,current.and_then(|v|v.2),m.encrypted_payload.as_ref().map(EncryptedPayload::as_opaque_bytes),m.timestamp,m.device_id.as_str(),m.operation_id.to_string()]).map_err(storage_error)?;
            tx.commit().map_err(storage_error)?;
            return Ok(ApplyOutcome::Conflict { conflict_id: id });
        }
        let deleted = matches!(m.kind, SyncOperationKind::Delete);
        tx.execute("INSERT INTO sync_entities(entity_id,entity_type,entity_revision,tombstone,payload,last_operation_id,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(entity_id) DO UPDATE SET entity_type=excluded.entity_type,entity_revision=excluded.entity_revision,tombstone=excluded.tombstone,payload=excluded.payload,last_operation_id=excluded.last_operation_id,updated_at=excluded.updated_at",params![m.entity_id.to_string(),entity_name(&m.entity_type),revision+1,deleted,if deleted{None}else{m.encrypted_payload.as_ref().map(EncryptedPayload::as_opaque_bytes)},m.operation_id.to_string(),m.timestamp]).map_err(storage_error)?;
        tx.commit().map_err(storage_error)?;
        Ok(ApplyOutcome::Applied { cursor: sequence })
    }
    pub fn checkpoint_and_close(self) -> Result<(), SyncError> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(storage_error)
    }
}
fn row_to_stored(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredSyncOperation> {
    let operation_id: String = r.get(1)?;
    let entity_id: String = r.get(2)?;
    let entity_type: String = r.get(3)?;
    let device: String = r.get(4)?;
    let kind: String = r.get(8)?;
    Ok(StoredSyncOperation {
        server_sequence: SyncCursor(r.get(0)?),
        mutation: SyncMutation {
            operation_id: Uuid::parse_str(&operation_id)
                .map_err(|_| rusqlite::Error::InvalidQuery)?,
            entity_id: Uuid::parse_str(&entity_id).map_err(|_| rusqlite::Error::InvalidQuery)?,
            entity_type: parse_entity(&entity_type)?,
            device_id: DeviceId::new(device).map_err(|_| rusqlite::Error::InvalidQuery)?,
            device_sequence: r.get(5)?,
            base_revision: r.get(6)?,
            kind: parse_kind(&kind)?,
            encrypted_payload: r
                .get::<_, Option<Vec<u8>>>(9)?
                .map(EncryptedPayload::from_opaque_bytes),
            schema_version: r.get(10)?,
            timestamp: r.get(11)?,
        },
        entity_revision: r.get(7)?,
        outcome: if r.get::<_, String>(12)? == "conflict" {
            StoredOperationOutcome::Conflict
        } else {
            StoredOperationOutcome::Applied
        },
    })
}
fn parse_uuid(v: &str) -> Result<Uuid, SyncError> {
    Uuid::parse_str(v).map_err(|_| SyncError::StorageUnavailable)
}
fn entity_name(v: &SyncEntityType) -> &'static str {
    match v {
        SyncEntityType::Note => "note",
        SyncEntityType::NoteFolder => "note_folder",
        SyncEntityType::NoteTag => "note_tag",
        SyncEntityType::AiMemory => "ai_memory",
        SyncEntityType::AutocorrectDictionary => "autocorrect_dictionary",
        SyncEntityType::UiSettings => "ui_settings",
        SyncEntityType::JarvisSettings => "jarvis_settings",
        SyncEntityType::AltronSettings => "altron_settings",
        SyncEntityType::VaultMetadata => "vault_metadata",
        SyncEntityType::VaultRecord => "vault_record",
    }
}
fn parse_entity(v: &str) -> Result<SyncEntityType, rusqlite::Error> {
    match v {
        "note" => Ok(SyncEntityType::Note),
        "note_folder" => Ok(SyncEntityType::NoteFolder),
        "note_tag" => Ok(SyncEntityType::NoteTag),
        "ai_memory" => Ok(SyncEntityType::AiMemory),
        "autocorrect_dictionary" => Ok(SyncEntityType::AutocorrectDictionary),
        "ui_settings" => Ok(SyncEntityType::UiSettings),
        "jarvis_settings" => Ok(SyncEntityType::JarvisSettings),
        "altron_settings" => Ok(SyncEntityType::AltronSettings),
        "vault_metadata" => Ok(SyncEntityType::VaultMetadata),
        "vault_record" => Ok(SyncEntityType::VaultRecord),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn kind_name(v: &SyncOperationKind) -> &'static str {
    match v {
        SyncOperationKind::Create => "create",
        SyncOperationKind::Update => "update",
        SyncOperationKind::Delete => "delete",
    }
}
fn parse_kind(v: &str) -> Result<SyncOperationKind, rusqlite::Error> {
    match v {
        "create" => Ok(SyncOperationKind::Create),
        "update" => Ok(SyncOperationKind::Update),
        "delete" => Ok(SyncOperationKind::Delete),
        _ => Err(rusqlite::Error::InvalidQuery),
    }
}
fn validate_mutation(m: &SyncMutation) -> Result<(), SyncError> {
    DeviceId::new(m.device_id.as_str())?;
    if m.device_sequence == 0 {
        return Err(SyncError::InvalidRevision);
    }
    if chrono::DateTime::parse_from_rfc3339(&m.timestamp).is_err() {
        return Err(SyncError::InvalidTimestamp);
    }
    if m.schema_version != 1 {
        return Err(SyncError::UnsupportedSchemaVersion);
    }
    if matches!(m.kind, SyncOperationKind::Delete) != m.encrypted_payload.is_none() {
        return Err(if matches!(m.kind, SyncOperationKind::Delete) {
            SyncError::InvalidTombstone
        } else {
            SyncError::MissingEncryptedPayload
        });
    }
    if m.encrypted_payload
        .as_ref()
        .is_some_and(|p| p.as_opaque_bytes().len() > MAX_ENCRYPTED_PAYLOAD_BYTES)
    {
        return Err(SyncError::PayloadTooLarge);
    }
    Ok(())
}
fn storage_error(_: SqlError) -> SyncError {
    SyncError::StorageUnavailable
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn mutation(
        id: Uuid,
        entity: Uuid,
        sequence: u64,
        base: u64,
        kind: SyncOperationKind,
    ) -> SyncMutation {
        SyncMutation {
            operation_id: id,
            entity_id: entity,
            entity_type: SyncEntityType::Note,
            device_id: DeviceId::new("sqlite_test_device").unwrap(),
            device_sequence: sequence,
            base_revision: base,
            kind: kind.clone(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            schema_version: 1,
            encrypted_payload: if matches!(kind, SyncOperationKind::Delete) {
                None
            } else {
                Some(EncryptedPayload::new(b"encrypted-fixture-only".to_vec()))
            },
        }
    }
    #[test]
    fn persists_idempotency_cursor_conflicts_and_tombstones() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("state.sqlite3");
        let entity = Uuid::new_v4();
        let first = mutation(Uuid::new_v4(), entity, 1, 0, SyncOperationKind::Create);
        {
            let mut repo = SqliteSyncRepository::open(&path).unwrap();
            assert_eq!(
                repo.apply_mutation(&first).unwrap(),
                ApplyOutcome::Applied { cursor: 1 }
            );
            assert_eq!(repo.record(entity).unwrap().unwrap().metadata.revision, 1);
            assert_eq!(repo.device_cursor(&first.device_id).unwrap(), SyncCursor(0));
            repo.save_device_cursor(&first.device_id, SyncCursor(1))
                .unwrap();
        }
        let mut repo = SqliteSyncRepository::open(&path).unwrap();
        assert_eq!(
            repo.apply_mutation(&first).unwrap(),
            ApplyOutcome::AlreadyApplied { cursor: 1 }
        );
        assert_eq!(repo.device_cursor(&first.device_id).unwrap(), SyncCursor(1));
        assert_eq!(repo.changes_after(SyncCursor(0), 1).unwrap().len(), 1);
        assert!(matches!(
            repo.apply_mutation(&mutation(
                Uuid::new_v4(),
                entity,
                2,
                0,
                SyncOperationKind::Update
            ))
            .unwrap(),
            ApplyOutcome::Conflict { .. }
        ));
        assert_eq!(repo.record(entity).unwrap().unwrap().metadata.revision, 1);
        assert_eq!(
            repo.apply_mutation(&mutation(
                Uuid::new_v4(),
                entity,
                3,
                1,
                SyncOperationKind::Delete
            ))
            .unwrap(),
            ApplyOutcome::Applied { cursor: 3 }
        );
        assert!(repo.record(entity).unwrap().unwrap().metadata.tombstone);
        assert_eq!(
            repo.apply_mutation(&mutation(
                Uuid::new_v4(),
                entity,
                4,
                2,
                SyncOperationKind::Update
            ))
            .unwrap(),
            ApplyOutcome::Rejected
        );
    }
    #[test]
    fn rejects_duplicate_device_sequence_without_writing_partial_state() {
        let directory = tempdir().unwrap();
        let mut repo = SqliteSyncRepository::open(&directory.path().join("state.sqlite3")).unwrap();
        let entity = Uuid::new_v4();
        assert!(matches!(
            repo.apply_mutation(&mutation(
                Uuid::new_v4(),
                entity,
                1,
                0,
                SyncOperationKind::Create
            )),
            Ok(ApplyOutcome::Applied { .. })
        ));
        assert_eq!(
            repo.apply_mutation(&mutation(
                Uuid::new_v4(),
                Uuid::new_v4(),
                1,
                0,
                SyncOperationKind::Create
            ))
            .unwrap(),
            ApplyOutcome::Rejected
        );
        assert_eq!(repo.changes_after(SyncCursor(0), 100).unwrap().len(), 1);
    }
}
