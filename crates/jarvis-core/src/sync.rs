//! Local-first synchronization contracts and conflict-safe operation handling.
//!
//! This module deliberately has no transport, database, pairing, or production
//! cryptography. Those integrations are separate stages; the only built-in
//! repository and crypto provider are compiled for tests only.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

const MAX_DEVICE_ID_LEN: usize = 128;
const MAX_PAGE_SIZE: usize = 100;
const MAX_ENCRYPTED_PAYLOAD_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncEntityType {
    Note,
    NoteFolder,
    NoteTag,
    AiMemory,
    AutocorrectDictionary,
    UiSettings,
    JarvisSettings,
    AltronSettings,
    VaultMetadata,
    VaultRecord,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOperationKind {
    Create,
    Update,
    Delete,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceId(String);

impl DeviceId {
    pub fn new(value: impl Into<String>) -> Result<Self, SyncError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_DEVICE_ID_LEN
            || !value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            return Err(SyncError::InvalidDeviceId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque bytes produced by a future production `CryptoProvider`.
///
/// Its `Debug` implementation intentionally never reveals bytes, so accidental
/// diagnostic output cannot expose notes or vault records.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct EncryptedPayload(Vec<u8>);

impl fmt::Debug for EncryptedPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EncryptedPayload(<redacted>)")
    }
}

impl EncryptedPayload {
    #[cfg(test)]
    fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct SyncRecordMetadata {
    pub id: Uuid,
    pub entity_type: SyncEntityType,
    pub revision: u64,
    pub device_id: DeviceId,
    pub updated_at: String,
    pub tombstone: bool,
}

impl fmt::Debug for SyncRecordMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyncRecordMetadata")
            .field("id", &self.id)
            .field("entity_type", &self.entity_type)
            .field("revision", &self.revision)
            .field("device_id", &self.device_id)
            .field("updated_at", &self.updated_at)
            .field("tombstone", &self.tombstone)
            .finish()
    }
}

/// Content stays separate from sync metadata. A tombstone has no content.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct SyncRecord {
    pub metadata: SyncRecordMetadata,
    pub content: Option<EncryptedPayload>,
}

impl fmt::Debug for SyncRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyncRecord")
            .field("metadata", &self.metadata)
            .field("content", &self.content.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct SyncOperation {
    pub id: Uuid,
    pub entity_id: Uuid,
    pub entity_type: SyncEntityType,
    pub base_revision: u64,
    pub revision: u64,
    pub device_id: DeviceId,
    pub kind: SyncOperationKind,
    pub tombstone: bool,
    pub updated_at: String,
    pub encrypted_payload: Option<EncryptedPayload>,
}

impl fmt::Debug for SyncOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyncOperation")
            .field("id", &self.id)
            .field("entity_id", &self.entity_id)
            .field("entity_type", &self.entity_type)
            .field("base_revision", &self.base_revision)
            .field("revision", &self.revision)
            .field("device_id", &self.device_id)
            .field("kind", &self.kind)
            .field("tombstone", &self.tombstone)
            .field("updated_at", &self.updated_at)
            .field(
                "encrypted_payload",
                &self.encrypted_payload.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SyncConflict {
    pub id: Uuid,
    pub entity_id: Uuid,
    pub entity_type: SyncEntityType,
    pub current: Option<SyncRecord>,
    pub incoming: SyncOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyOutcome {
    Applied { cursor: u64 },
    AlreadyApplied { cursor: u64 },
    Conflict { conflict_id: Uuid },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncPage {
    pub operations: Vec<SyncOperation>,
    pub next_cursor: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncAuditEntry {
    pub operation_id: Uuid,
    pub entity_id: Uuid,
    pub entity_type: SyncEntityType,
    pub outcome: SyncAuditOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncAuditOutcome {
    Applied,
    AlreadyApplied,
    Conflict,
}

pub trait SyncRepository {
    fn record(&self, entity_id: Uuid) -> Option<SyncRecord>;
    fn save_record(&mut self, record: SyncRecord);
    fn applied_outcome(&self, operation_id: Uuid) -> Option<ApplyOutcome>;
    fn save_applied_outcome(&mut self, operation_id: Uuid, outcome: ApplyOutcome);
    fn append_change(&mut self, operation: SyncOperation) -> u64;
    fn changes_after(&self, cursor: u64, limit: usize) -> SyncPage;
    fn save_conflict(&mut self, conflict: SyncConflict);
    fn conflicts(&self) -> Vec<SyncConflict>;
    fn append_audit(&mut self, entry: SyncAuditEntry);
    fn audit_entries(&self) -> Vec<SyncAuditEntry>;
}

pub trait CryptoProvider {
    fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedPayload, SyncError>;
    fn decrypt(&self, payload: &EncryptedPayload) -> Result<Vec<u8>, SyncError>;
}

pub struct SyncEngine<R, C> {
    repository: R,
    crypto: C,
    device_id: DeviceId,
}

impl<R: SyncRepository, C: CryptoProvider> SyncEngine<R, C> {
    pub fn new(repository: R, crypto: C, device_id: DeviceId) -> Self {
        Self {
            repository,
            crypto,
            device_id,
        }
    }

    pub fn create_local_change(
        &mut self,
        entity_type: SyncEntityType,
        entity_id: Uuid,
        plaintext: &[u8],
    ) -> Result<SyncOperation, SyncError> {
        let base_revision = self
            .repository
            .record(entity_id)
            .map(|record| record.metadata.revision)
            .unwrap_or(0);
        let kind = if base_revision == 0 {
            SyncOperationKind::Create
        } else {
            SyncOperationKind::Update
        };
        let operation = SyncOperation {
            id: Uuid::new_v4(),
            entity_id,
            entity_type,
            base_revision,
            revision: base_revision + 1,
            device_id: self.device_id.clone(),
            kind,
            tombstone: false,
            updated_at: Utc::now().to_rfc3339(),
            encrypted_payload: Some(self.crypto.encrypt(plaintext)?),
        };
        self.apply_operation(operation.clone())?;
        Ok(operation)
    }

    pub fn create_local_delete(
        &mut self,
        entity_type: SyncEntityType,
        entity_id: Uuid,
    ) -> Result<SyncOperation, SyncError> {
        let base_revision = self
            .repository
            .record(entity_id)
            .map(|record| record.metadata.revision)
            .unwrap_or(0);
        let operation = SyncOperation {
            id: Uuid::new_v4(),
            entity_id,
            entity_type,
            base_revision,
            revision: base_revision + 1,
            device_id: self.device_id.clone(),
            kind: SyncOperationKind::Delete,
            tombstone: true,
            updated_at: Utc::now().to_rfc3339(),
            encrypted_payload: None,
        };
        self.apply_operation(operation.clone())?;
        Ok(operation)
    }

    pub fn apply_operation(&mut self, operation: SyncOperation) -> Result<ApplyOutcome, SyncError> {
        validate_operation(&operation)?;

        if let Some(outcome) = self.repository.applied_outcome(operation.id) {
            self.repository.append_audit(SyncAuditEntry {
                operation_id: operation.id,
                entity_id: operation.entity_id,
                entity_type: operation.entity_type,
                outcome: SyncAuditOutcome::AlreadyApplied,
            });
            return Ok(match outcome {
                ApplyOutcome::Applied { cursor } | ApplyOutcome::AlreadyApplied { cursor } => {
                    ApplyOutcome::AlreadyApplied { cursor }
                }
                conflict @ ApplyOutcome::Conflict { .. } => conflict,
            });
        }

        let current = self.repository.record(operation.entity_id);
        let current_revision = current
            .as_ref()
            .map(|record| record.metadata.revision)
            .unwrap_or(0);

        if current_revision != operation.base_revision {
            let conflict = SyncConflict {
                id: Uuid::new_v4(),
                entity_id: operation.entity_id,
                entity_type: operation.entity_type.clone(),
                current,
                incoming: operation.clone(),
            };
            let outcome = ApplyOutcome::Conflict {
                conflict_id: conflict.id,
            };
            self.repository.save_conflict(conflict);
            self.repository
                .save_applied_outcome(operation.id, outcome.clone());
            self.repository.append_audit(SyncAuditEntry {
                operation_id: operation.id,
                entity_id: operation.entity_id,
                entity_type: operation.entity_type,
                outcome: SyncAuditOutcome::Conflict,
            });
            return Ok(outcome);
        }

        let record = SyncRecord {
            metadata: SyncRecordMetadata {
                id: operation.entity_id,
                entity_type: operation.entity_type.clone(),
                revision: operation.revision,
                device_id: operation.device_id.clone(),
                updated_at: operation.updated_at.clone(),
                tombstone: operation.tombstone,
            },
            content: operation.encrypted_payload.clone(),
        };
        self.repository.save_record(record);
        let cursor = self.repository.append_change(operation.clone());
        let outcome = ApplyOutcome::Applied { cursor };
        self.repository
            .save_applied_outcome(operation.id, outcome.clone());
        self.repository.append_audit(SyncAuditEntry {
            operation_id: operation.id,
            entity_id: operation.entity_id,
            entity_type: operation.entity_type,
            outcome: SyncAuditOutcome::Applied,
        });
        Ok(outcome)
    }

    pub fn changes_after(&self, cursor: u64, limit: usize) -> SyncPage {
        self.repository
            .changes_after(cursor, limit.min(MAX_PAGE_SIZE))
    }

    pub fn conflicts(&self) -> Vec<SyncConflict> {
        self.repository.conflicts()
    }

    pub fn audit_entries(&self) -> Vec<SyncAuditEntry> {
        self.repository.audit_entries()
    }

    pub fn record(&self, entity_id: Uuid) -> Option<SyncRecord> {
        self.repository.record(entity_id)
    }
}

fn validate_operation(operation: &SyncOperation) -> Result<(), SyncError> {
    DeviceId::new(operation.device_id.as_str())?;
    if operation.revision != operation.base_revision + 1 {
        return Err(SyncError::InvalidRevision);
    }
    if matches!(operation.kind, SyncOperationKind::Create) != (operation.base_revision == 0) {
        return Err(SyncError::InvalidOperationKind);
    }
    if chrono::DateTime::parse_from_rfc3339(&operation.updated_at).is_err() {
        return Err(SyncError::InvalidTimestamp);
    }
    if operation.tombstone != matches!(operation.kind, SyncOperationKind::Delete) {
        return Err(SyncError::InvalidTombstone);
    }
    if operation.tombstone && operation.encrypted_payload.is_some() {
        return Err(SyncError::InvalidTombstone);
    }
    if !operation.tombstone && operation.encrypted_payload.is_none() {
        return Err(SyncError::MissingEncryptedPayload);
    }
    if operation
        .encrypted_payload
        .as_ref()
        .is_some_and(|payload| payload.0.len() > MAX_ENCRYPTED_PAYLOAD_BYTES)
    {
        return Err(SyncError::PayloadTooLarge);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncError {
    InvalidDeviceId,
    InvalidRevision,
    InvalidOperationKind,
    InvalidTimestamp,
    InvalidTombstone,
    MissingEncryptedPayload,
    PayloadTooLarge,
    CryptoUnavailable,
    TestPayloadMalformed,
}

impl fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDeviceId => formatter.write_str("invalid sync device identifier"),
            Self::InvalidRevision => formatter.write_str("invalid sync revision"),
            Self::InvalidOperationKind => formatter.write_str("invalid sync operation kind"),
            Self::InvalidTimestamp => formatter.write_str("invalid sync timestamp"),
            Self::InvalidTombstone => formatter.write_str("invalid sync tombstone"),
            Self::MissingEncryptedPayload => formatter.write_str("missing encrypted sync payload"),
            Self::PayloadTooLarge => formatter.write_str("encrypted sync payload exceeds limit"),
            Self::CryptoUnavailable => formatter.write_str("sync crypto provider unavailable"),
            Self::TestPayloadMalformed => formatter.write_str("malformed test-only sync payload"),
        }
    }
}

impl std::error::Error for SyncError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct InMemorySyncRepository {
        records: HashMap<Uuid, SyncRecord>,
        outcomes: HashMap<Uuid, ApplyOutcome>,
        changes: Vec<SyncOperation>,
        conflicts: Vec<SyncConflict>,
        audit: Vec<SyncAuditEntry>,
    }

    impl SyncRepository for InMemorySyncRepository {
        fn record(&self, entity_id: Uuid) -> Option<SyncRecord> {
            self.records.get(&entity_id).cloned()
        }

        fn save_record(&mut self, record: SyncRecord) {
            self.records.insert(record.metadata.id, record);
        }

        fn applied_outcome(&self, operation_id: Uuid) -> Option<ApplyOutcome> {
            self.outcomes.get(&operation_id).cloned()
        }

        fn save_applied_outcome(&mut self, operation_id: Uuid, outcome: ApplyOutcome) {
            self.outcomes.insert(operation_id, outcome);
        }

        fn append_change(&mut self, operation: SyncOperation) -> u64 {
            self.changes.push(operation);
            self.changes.len() as u64
        }

        fn changes_after(&self, cursor: u64, limit: usize) -> SyncPage {
            let start = usize::try_from(cursor).unwrap_or(usize::MAX);
            let operations = self
                .changes
                .iter()
                .skip(start)
                .take(limit)
                .cloned()
                .collect::<Vec<_>>();
            let next_cursor = cursor + operations.len() as u64;
            SyncPage {
                operations,
                next_cursor,
            }
        }

        fn save_conflict(&mut self, conflict: SyncConflict) {
            self.conflicts.push(conflict);
        }

        fn conflicts(&self) -> Vec<SyncConflict> {
            self.conflicts.clone()
        }

        fn append_audit(&mut self, entry: SyncAuditEntry) {
            self.audit.push(entry);
        }

        fn audit_entries(&self) -> Vec<SyncAuditEntry> {
            self.audit.clone()
        }
    }

    /// A reversible fixture, not encryption. It exists only in `cfg(test)` and
    /// must never be enabled in a production configuration.
    #[derive(Default)]
    struct TestOnlyCryptoProvider;

    impl CryptoProvider for TestOnlyCryptoProvider {
        fn encrypt(&self, plaintext: &[u8]) -> Result<EncryptedPayload, SyncError> {
            let mut bytes = b"test-only:".to_vec();
            bytes.extend(plaintext.iter().rev());
            Ok(EncryptedPayload::new(bytes))
        }

        fn decrypt(&self, payload: &EncryptedPayload) -> Result<Vec<u8>, SyncError> {
            let bytes = payload
                .0
                .strip_prefix(b"test-only:")
                .ok_or(SyncError::TestPayloadMalformed)?;
            Ok(bytes.iter().rev().copied().collect())
        }
    }

    fn engine(device_id: &str) -> SyncEngine<InMemorySyncRepository, TestOnlyCryptoProvider> {
        SyncEngine::new(
            InMemorySyncRepository::default(),
            TestOnlyCryptoProvider,
            DeviceId::new(device_id).unwrap(),
        )
    }

    fn remote_update(
        device_id: &str,
        entity_id: Uuid,
        base_revision: u64,
        plaintext: &[u8],
    ) -> SyncOperation {
        let crypto = TestOnlyCryptoProvider;
        SyncOperation {
            id: Uuid::new_v4(),
            entity_id,
            entity_type: SyncEntityType::Note,
            base_revision,
            revision: base_revision + 1,
            device_id: DeviceId::new(device_id).unwrap(),
            kind: if base_revision == 0 {
                SyncOperationKind::Create
            } else {
                SyncOperationKind::Update
            },
            tombstone: false,
            updated_at: Utc::now().to_rfc3339(),
            encrypted_payload: Some(crypto.encrypt(plaintext).unwrap()),
        }
    }

    #[test]
    fn creates_a_local_operation_and_separates_content_from_metadata() {
        let mut engine = engine("desktop");
        let entity_id = Uuid::new_v4();
        let operation = engine
            .create_local_change(SyncEntityType::Note, entity_id, b"fictional note")
            .unwrap();

        assert_eq!(operation.base_revision, 0);
        assert_eq!(operation.revision, 1);
        assert_eq!(operation.device_id.as_str(), "desktop");
        assert!(operation.encrypted_payload.is_some());
        let record = engine.record(entity_id).unwrap();
        assert_eq!(record.metadata.revision, 1);
        assert_ne!(format!("{record:?}"), "fictional note");
    }

    #[test]
    fn repeated_operation_is_idempotent() {
        let mut engine = engine("desktop");
        let operation = engine
            .create_local_change(SyncEntityType::Note, Uuid::new_v4(), b"fixture")
            .unwrap();

        assert_eq!(
            engine.apply_operation(operation).unwrap(),
            ApplyOutcome::AlreadyApplied { cursor: 1 }
        );
    }

    #[test]
    fn sequential_updates_advance_the_revision() {
        let mut engine = engine("desktop");
        let entity_id = Uuid::new_v4();
        engine
            .create_local_change(SyncEntityType::Note, entity_id, b"first")
            .unwrap();
        let update = engine
            .create_local_change(SyncEntityType::Note, entity_id, b"second")
            .unwrap();

        assert_eq!(update.base_revision, 1);
        assert_eq!(update.revision, 2);
        assert_eq!(engine.record(entity_id).unwrap().metadata.revision, 2);
    }

    #[test]
    fn base_revision_mismatch_creates_a_conflict() {
        let mut engine = engine("desktop");
        let entity_id = Uuid::new_v4();
        engine
            .create_local_change(SyncEntityType::Note, entity_id, b"current")
            .unwrap();
        let stale = remote_update("android", entity_id, 0, b"stale");

        assert!(matches!(
            engine.apply_operation(stale),
            Ok(ApplyOutcome::Conflict { .. })
        ));
        assert_eq!(engine.record(entity_id).unwrap().metadata.revision, 1);
    }

    #[test]
    fn conflicts_preserve_both_versions() {
        let mut engine = engine("desktop");
        let entity_id = Uuid::new_v4();
        engine
            .create_local_change(SyncEntityType::Note, entity_id, b"desktop version")
            .unwrap();
        let incoming = remote_update("android", entity_id, 0, b"android version");
        engine.apply_operation(incoming.clone()).unwrap();

        let conflict = engine.conflicts().pop().unwrap();
        assert_eq!(conflict.current.unwrap().metadata.revision, 1);
        assert_eq!(conflict.incoming.id, incoming.id);
        assert_eq!(
            TestOnlyCryptoProvider
                .decrypt(conflict.incoming.encrypted_payload.as_ref().unwrap())
                .unwrap(),
            b"android version"
        );
    }

    #[test]
    fn tombstone_is_recorded_and_repeated_delete_is_idempotent() {
        let mut engine = engine("desktop");
        let entity_id = Uuid::new_v4();
        engine
            .create_local_change(SyncEntityType::Note, entity_id, b"fixture")
            .unwrap();
        let delete = engine
            .create_local_delete(SyncEntityType::Note, entity_id)
            .unwrap();

        let record = engine.record(entity_id).unwrap();
        assert!(record.metadata.tombstone);
        assert!(record.content.is_none());
        assert_eq!(
            engine.apply_operation(delete).unwrap(),
            ApplyOutcome::AlreadyApplied { cursor: 2 }
        );
    }

    #[test]
    fn sequential_tombstones_do_not_resurrect_the_record() {
        let mut engine = engine("desktop");
        let entity_id = Uuid::new_v4();
        engine
            .create_local_change(SyncEntityType::Note, entity_id, b"fixture")
            .unwrap();
        engine
            .create_local_delete(SyncEntityType::Note, entity_id)
            .unwrap();
        engine
            .create_local_delete(SyncEntityType::Note, entity_id)
            .unwrap();

        let record = engine.record(entity_id).unwrap();
        assert!(record.metadata.tombstone);
        assert_eq!(record.metadata.revision, 3);
    }

    #[test]
    fn operations_from_different_devices_are_retained() {
        let mut engine = engine("desktop");
        let entity_id = Uuid::new_v4();
        engine
            .create_local_change(SyncEntityType::Note, entity_id, b"fixture")
            .unwrap();
        let remote = remote_update("android_phone", entity_id, 1, b"remote");

        engine.apply_operation(remote).unwrap();
        assert_eq!(
            engine
                .record(entity_id)
                .unwrap()
                .metadata
                .device_id
                .as_str(),
            "android_phone"
        );
    }

    #[test]
    fn changes_are_returned_after_a_sync_cursor() {
        let mut engine = engine("desktop");
        let first_id = Uuid::new_v4();
        engine
            .create_local_change(SyncEntityType::Note, first_id, b"first")
            .unwrap();
        engine
            .create_local_change(SyncEntityType::Note, Uuid::new_v4(), b"second")
            .unwrap();

        let page = engine.changes_after(1, 10);
        assert_eq!(page.operations.len(), 1);
        assert_eq!(page.next_cursor, 2);
        assert_ne!(page.operations[0].entity_id, first_id);
    }

    #[test]
    fn audit_and_debug_output_never_include_note_or_password_content() {
        let mut engine = engine("desktop");
        let secret_note = "FICTIONAL_NOTE_CONTENT_DO_NOT_LOG";
        let fake_password = "FICTIONAL_PASSWORD_DO_NOT_LOG";
        let operation = engine
            .create_local_change(
                SyncEntityType::VaultRecord,
                Uuid::new_v4(),
                format!("{secret_note}:{fake_password}").as_bytes(),
            )
            .unwrap();

        let audit = format!("{:?}", engine.audit_entries());
        let operation_debug = format!("{operation:?}");
        assert!(!audit.contains(secret_note));
        assert!(!audit.contains(fake_password));
        assert!(!operation_debug.contains(secret_note));
        assert!(!operation_debug.contains(fake_password));
    }
}
