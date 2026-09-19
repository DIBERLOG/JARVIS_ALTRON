//! Local-first synchronization contracts and conflict-safe operation handling.
//!
//! This module has no transport, pairing, or network code. Clients submit
//! [`SyncMutation`] values and the repository is the only authority that assigns
//! entity revisions and monotonic server sequences. Record content is always an
//! opaque [`EncryptedPayload`]: the encryption itself lives in [`crypto`], and
//! durable storage lives in [`sqlite`].
//!
//! Ordering never depends on wall-clock timestamps. The only ordering source is
//! the repository-assigned server sequence ([`SyncCursor`]).

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

pub mod crypto;
pub mod sqlite;

const MAX_DEVICE_ID_LEN: usize = 128;
pub(crate) const MAX_PAGE_SIZE: usize = 100;
pub(crate) const MAX_ENCRYPTED_PAYLOAD_BYTES: usize = 1024 * 1024;
const CONFLICT_REASON_BASE_REVISION: &str = "base_revision_mismatch";

/// Every synced entity type. Adding a variant requires a storage review because
/// the name becomes part of the persisted journal.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncEntityType {
    Note,
    NoteFolder,
    NoteTag,
    /// Reserved for AI-memory data that is not one of the four kinds below.
    AiMemory,
    /// One conversation header.
    AiMemoryConversation,
    /// One stored chat message.
    AiMemoryMessage,
    /// One conversation summary.
    AiMemorySummary,
    /// One long-term fact, preference, or candidate.
    AiMemoryFact,
    AutocorrectDictionary,
    UiSettings,
    JarvisSettings,
    AltronSettings,
    VaultMetadata,
    VaultRecord,
}

impl SyncEntityType {
    /// Stable storage name. Never reuse an existing string for a new variant.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::NoteFolder => "note_folder",
            Self::NoteTag => "note_tag",
            Self::AiMemory => "ai_memory",
            Self::AiMemoryConversation => "ai_memory_conversation",
            Self::AiMemoryMessage => "ai_memory_message",
            Self::AiMemorySummary => "ai_memory_summary",
            Self::AiMemoryFact => "ai_memory_fact",
            Self::AutocorrectDictionary => "autocorrect_dictionary",
            Self::UiSettings => "ui_settings",
            Self::JarvisSettings => "jarvis_settings",
            Self::AltronSettings => "altron_settings",
            Self::VaultMetadata => "vault_metadata",
            Self::VaultRecord => "vault_record",
        }
    }

    pub fn from_storage_name(value: &str) -> Result<Self, SyncError> {
        match value {
            "note" => Ok(Self::Note),
            "note_folder" => Ok(Self::NoteFolder),
            "note_tag" => Ok(Self::NoteTag),
            "ai_memory" => Ok(Self::AiMemory),
            "ai_memory_conversation" => Ok(Self::AiMemoryConversation),
            "ai_memory_message" => Ok(Self::AiMemoryMessage),
            "ai_memory_summary" => Ok(Self::AiMemorySummary),
            "ai_memory_fact" => Ok(Self::AiMemoryFact),
            "autocorrect_dictionary" => Ok(Self::AutocorrectDictionary),
            "ui_settings" => Ok(Self::UiSettings),
            "jarvis_settings" => Ok(Self::JarvisSettings),
            "altron_settings" => Ok(Self::AltronSettings),
            "vault_metadata" => Ok(Self::VaultMetadata),
            "vault_record" => Ok(Self::VaultRecord),
            _ => Err(SyncError::StorageCorrupt),
        }
    }

    /// Whether this type belongs to the encrypted AI memory.
    ///
    /// The memory store only ever reads and writes these types, which is one of
    /// the reasons it cannot reach notes or password records even with a wrong
    /// entity identifier.
    pub fn is_ai_memory(&self) -> bool {
        matches!(
            self,
            Self::AiMemory
                | Self::AiMemoryConversation
                | Self::AiMemoryMessage
                | Self::AiMemorySummary
                | Self::AiMemoryFact
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncOperationKind {
    Create,
    Update,
    Delete,
}

impl SyncOperationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }

    pub fn from_storage_name(value: &str) -> Result<Self, SyncError> {
        match value {
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "delete" => Ok(Self::Delete),
            _ => Err(SyncError::StorageCorrupt),
        }
    }
}

/// A validated device identifier. Device IDs appear in storage metadata and are
/// never secret, so they stay readable while payloads do not.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
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

/// Opaque bytes produced by a [`CryptoProvider`].
///
/// Its `Debug` implementation intentionally never reveals bytes, so accidental
/// diagnostic output cannot expose notes, AI memory, or vault records.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct EncryptedPayload(Vec<u8>);

impl fmt::Debug for EncryptedPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EncryptedPayload(<redacted>)")
    }
}

impl EncryptedPayload {
    /// Wraps ciphertext produced by this crate's crypto layer.
    pub(crate) fn from_opaque_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub(crate) fn as_opaque_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Length in bytes; safe to log because it reveals no content.
    pub fn byte_len(&self) -> usize {
        self.0.len()
    }
}

/// Identifies the logical location a payload belongs to.
///
/// It is authenticated as associated data, so ciphertext moved to another
/// entity type or entity ID fails to decrypt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadContext {
    pub entity_type: SyncEntityType,
    pub entity_id: Uuid,
}

impl PayloadContext {
    pub fn new(entity_type: SyncEntityType, entity_id: Uuid) -> Self {
        Self {
            entity_type,
            entity_id,
        }
    }

    pub fn aad(&self) -> Vec<u8> {
        let name = self.entity_type.as_str().as_bytes();
        let mut aad = Vec::with_capacity(name.len() + 1 + 16);
        aad.extend_from_slice(name);
        aad.push(0);
        aad.extend_from_slice(self.entity_id.as_bytes());
        aad
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

/// A client-originated request.
///
/// It deliberately cannot carry server-assigned entity revisions or journal
/// positions. `schema_version` describes the payload format, not the storage
/// schema.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct SyncMutation {
    pub operation_id: Uuid,
    pub entity_id: Uuid,
    pub entity_type: SyncEntityType,
    pub device_id: DeviceId,
    pub device_sequence: u64,
    pub base_revision: u64,
    pub kind: SyncOperationKind,
    pub timestamp: String,
    pub schema_version: u32,
    pub encrypted_payload: Option<EncryptedPayload>,
}

impl SyncMutation {
    /// Current client payload schema version.
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;
}

impl fmt::Debug for SyncMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SyncMutation")
            .field("operation_id", &self.operation_id)
            .field("entity_id", &self.entity_id)
            .field("entity_type", &self.entity_type)
            .field("device_id", &self.device_id)
            .field("device_sequence", &self.device_sequence)
            .field("base_revision", &self.base_revision)
            .field("kind", &self.kind)
            .field("timestamp", &self.timestamp)
            .field("schema_version", &self.schema_version)
            .field(
                "encrypted_payload",
                &self.encrypted_payload.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// Monotonic repository-assigned position in the journal.
///
/// The cursor is not an entity revision and not a timestamp. It only orders
/// journal entries, so two devices can compare progress without trusting clocks.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct SyncCursor(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoredOperationOutcome {
    /// The mutation changed the entity.
    Applied,
    /// The mutation was recorded but the entity was left untouched.
    Conflict,
}

/// One journal entry: the request as submitted plus what the repository decided.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSyncOperation {
    pub mutation: SyncMutation,
    /// Revision the entity holds as a result of this operation. For a conflict
    /// the entity keeps the revision it already had.
    pub entity_revision: u64,
    pub server_sequence: SyncCursor,
    pub outcome: StoredOperationOutcome,
    pub conflict_id: Option<Uuid>,
}

/// Why an incoming version was preserved instead of applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictReason {
    /// The entity revision on record differs from the mutation's base revision.
    BaseRevisionMismatch,
}

impl ConflictReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BaseRevisionMismatch => CONFLICT_REASON_BASE_REVISION,
        }
    }

    pub fn from_storage_name(value: &str) -> Result<Self, SyncError> {
        match value {
            CONFLICT_REASON_BASE_REVISION => Ok(Self::BaseRevisionMismatch),
            _ => Err(SyncError::StorageCorrupt),
        }
    }
}

/// A rejected-but-retained incoming version, with the entity state it lost to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MutationConflict {
    pub id: Uuid,
    pub entity_id: Uuid,
    pub entity_type: SyncEntityType,
    pub reason: ConflictReason,
    /// Entity state at detection time; `None` when the entity did not exist.
    pub current: Option<SyncRecord>,
    pub incoming: SyncMutation,
    pub entity_revision: u64,
    pub server_sequence: SyncCursor,
}

/// Result of submitting one mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyOutcome {
    /// Applied now; `cursor` is the journal position assigned to it.
    Applied { cursor: SyncCursor },
    /// This exact operation ID was already stored, so nothing changed and the
    /// cursor was not advanced.
    AlreadyApplied { cursor: SyncCursor },
    /// The current entity revision differs from the base revision. The entity
    /// was not modified and the incoming version was stored as a conflict.
    Conflict { conflict_id: Uuid },
    /// The mutation was refused without writing anything: a device sequence was
    /// reused for a different operation, or the entity is a tombstone that the
    /// mutation tried to resurrect.
    Rejected,
}

/// A page of applied journal entries plus the cursor to resume from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncPage {
    /// Applied operations only. Conflicts are never replicated to peers.
    pub operations: Vec<StoredSyncOperation>,
    /// Cursor to pass to the next page request. Advances past conflicts too.
    pub next_cursor: SyncCursor,
}

/// Encrypts and decrypts payloads. Implemented by
/// [`crypto::MasterKeyCryptoProvider`]; tests may supply a harmless fixture.
pub trait CryptoProvider {
    fn encrypt(
        &self,
        context: &PayloadContext,
        plaintext: &[u8],
    ) -> Result<EncryptedPayload, SyncError>;

    fn decrypt(
        &self,
        context: &PayloadContext,
        payload: &EncryptedPayload,
    ) -> Result<Vec<u8>, SyncError>;
}

/// Durable or in-memory sync state.
///
/// [`apply_mutation`](SyncRepository::apply_mutation) is the single write
/// entry point. An implementation must perform all of its checks and writes in
/// one transaction: idempotency check, device-sequence check, current entity
/// read, base-revision check, revision and server-sequence assignment, journal
/// write, and the entity or conflict write. It must never leave partial state.
pub trait SyncRepository {
    fn apply_mutation(&mut self, mutation: SyncMutation) -> Result<ApplyOutcome, SyncError>;

    /// Current entity state, or `None` when the entity is unknown.
    fn record(&self, entity_id: Uuid) -> Result<Option<SyncRecord>, SyncError>;

    /// Applied entries after `cursor`, at most `limit` of them.
    fn page_after(&self, cursor: SyncCursor, limit: usize) -> Result<SyncPage, SyncError>;

    /// Retained conflicts, ordered by the journal position where they occurred.
    fn conflicts(&self) -> Result<Vec<MutationConflict>, SyncError>;

    /// Highest server sequence this device has already pulled.
    fn device_cursor(&self, device: &DeviceId) -> Result<SyncCursor, SyncError>;

    /// Stores a device's pull position.
    fn save_device_cursor(
        &mut self,
        device: &DeviceId,
        cursor: SyncCursor,
    ) -> Result<(), SyncError>;

    /// Highest device sequence already used by this device, or 0.
    fn last_device_sequence(&self, device: &DeviceId) -> Result<u64, SyncError>;

    /// Forgets a retained conflict after an operator resolved it.
    ///
    /// Returns whether the conflict existed. The journal entry is deliberately
    /// kept, so the incoming version is still auditable afterwards: resolving a
    /// conflict never destroys a version silently.
    fn discard_conflict(&mut self, conflict_id: Uuid) -> Result<bool, SyncError>;
}

/// Ephemeral repository used by tests and by short-lived, non-persistent flows.
///
/// It implements exactly the same conflict, revision, cursor, and idempotency
/// rules as the SQLite repository, but nothing survives the process. Never use
/// it for production notes or vault data.
#[derive(Default)]
pub struct InMemorySyncRepository {
    entities: HashMap<Uuid, SyncRecord>,
    outcomes: HashMap<Uuid, ApplyOutcome>,
    journal: Vec<StoredSyncOperation>,
    conflicts: Vec<MutationConflict>,
    device_sequences: HashMap<(DeviceId, u64), Uuid>,
    device_cursors: HashMap<DeviceId, SyncCursor>,
    last_device_sequences: HashMap<DeviceId, u64>,
    last_server_sequence: u64,
}

impl InMemorySyncRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SyncRepository for InMemorySyncRepository {
    fn apply_mutation(&mut self, mutation: SyncMutation) -> Result<ApplyOutcome, SyncError> {
        validate_mutation(&mutation)?;

        // A repeated operation ID is idempotent and never advances the cursor.
        if let Some(stored) = self.outcomes.get(&mutation.operation_id) {
            return Ok(match stored {
                ApplyOutcome::Applied { cursor } | ApplyOutcome::AlreadyApplied { cursor } => {
                    ApplyOutcome::AlreadyApplied { cursor: *cursor }
                }
                other => other.clone(),
            });
        }
        // A device sequence may be spent once. Nothing is recorded, so a retry
        // yields the same decision.
        let device_sequence = (mutation.device_id.clone(), mutation.device_sequence);
        if self.device_sequences.contains_key(&device_sequence) {
            return Ok(ApplyOutcome::Rejected);
        }

        let current = self.entities.get(&mutation.entity_id).cloned();
        let current_revision = current
            .as_ref()
            .map(|record| record.metadata.revision)
            .unwrap_or(0);

        // A conflict is detected before the tombstone guard: the base revision
        // is authoritative for deciding whether the entity may be touched.
        if current_revision != mutation.base_revision {
            let server_sequence = SyncCursor(self.last_server_sequence + 1);
            let conflict = MutationConflict {
                id: Uuid::new_v4(),
                entity_id: mutation.entity_id,
                entity_type: mutation.entity_type.clone(),
                reason: ConflictReason::BaseRevisionMismatch,
                current,
                incoming: mutation.clone(),
                entity_revision: current_revision,
                server_sequence,
            };
            let outcome = ApplyOutcome::Conflict {
                conflict_id: conflict.id,
            };
            self.last_server_sequence = server_sequence.0;
            self.journal.push(StoredSyncOperation {
                mutation: mutation.clone(),
                entity_revision: current_revision,
                server_sequence,
                outcome: StoredOperationOutcome::Conflict,
                conflict_id: Some(conflict.id),
            });
            self.conflicts.push(conflict);
            self.outcomes.insert(mutation.operation_id, outcome.clone());
            self.device_sequences
                .insert(device_sequence, mutation.operation_id);
            self.remember_device_sequence(&mutation);
            return Ok(outcome);
        }

        // A tombstone can only be replaced by another delete at the next revision.
        if current
            .as_ref()
            .is_some_and(|record| record.metadata.tombstone)
            && !matches!(mutation.kind, SyncOperationKind::Delete)
        {
            return Ok(ApplyOutcome::Rejected);
        }

        let entity_revision = current_revision + 1;
        let tombstone = matches!(mutation.kind, SyncOperationKind::Delete);
        let record = SyncRecord {
            metadata: SyncRecordMetadata {
                id: mutation.entity_id,
                entity_type: mutation.entity_type.clone(),
                revision: entity_revision,
                device_id: mutation.device_id.clone(),
                updated_at: mutation.timestamp.clone(),
                tombstone,
            },
            content: if tombstone {
                None
            } else {
                mutation.encrypted_payload.clone()
            },
        };
        let server_sequence = SyncCursor(self.last_server_sequence + 1);
        self.last_server_sequence = server_sequence.0;
        self.entities.insert(mutation.entity_id, record);
        self.journal.push(StoredSyncOperation {
            mutation: mutation.clone(),
            entity_revision,
            server_sequence,
            outcome: StoredOperationOutcome::Applied,
            conflict_id: None,
        });
        let outcome = ApplyOutcome::Applied {
            cursor: server_sequence,
        };
        self.outcomes.insert(mutation.operation_id, outcome.clone());
        self.device_sequences
            .insert(device_sequence, mutation.operation_id);
        self.remember_device_sequence(&mutation);
        Ok(outcome)
    }

    fn record(&self, entity_id: Uuid) -> Result<Option<SyncRecord>, SyncError> {
        Ok(self.entities.get(&entity_id).cloned())
    }

    fn page_after(&self, cursor: SyncCursor, limit: usize) -> Result<SyncPage, SyncError> {
        let limit = limit.min(MAX_PAGE_SIZE);
        if limit == 0 {
            return Ok(SyncPage {
                operations: Vec::new(),
                next_cursor: cursor,
            });
        }
        let mut next = cursor.0;
        let mut operations = Vec::new();
        for stored in self
            .journal
            .iter()
            .filter(|stored| stored.server_sequence.0 > cursor.0)
        {
            next = stored.server_sequence.0;
            if matches!(stored.outcome, StoredOperationOutcome::Applied) {
                operations.push(stored.clone());
            }
            // Conflicts advance the cursor without being replicated.
            if operations.len() >= limit {
                return Ok(SyncPage {
                    operations,
                    next_cursor: SyncCursor(next),
                });
            }
        }
        Ok(SyncPage {
            operations,
            next_cursor: SyncCursor(next),
        })
    }

    fn conflicts(&self) -> Result<Vec<MutationConflict>, SyncError> {
        Ok(self.conflicts.clone())
    }

    fn device_cursor(&self, device: &DeviceId) -> Result<SyncCursor, SyncError> {
        Ok(self
            .device_cursors
            .get(device)
            .copied()
            .unwrap_or(SyncCursor(0)))
    }

    fn save_device_cursor(
        &mut self,
        device: &DeviceId,
        cursor: SyncCursor,
    ) -> Result<(), SyncError> {
        self.device_cursors.insert(device.clone(), cursor);
        Ok(())
    }

    fn last_device_sequence(&self, device: &DeviceId) -> Result<u64, SyncError> {
        Ok(self
            .last_device_sequences
            .get(device)
            .copied()
            .unwrap_or(0))
    }

    fn discard_conflict(&mut self, conflict_id: Uuid) -> Result<bool, SyncError> {
        let before = self.conflicts.len();
        self.conflicts.retain(|conflict| conflict.id != conflict_id);
        Ok(self.conflicts.len() != before)
    }
}

impl InMemorySyncRepository {
    fn remember_device_sequence(&mut self, mutation: &SyncMutation) {
        let entry = self
            .last_device_sequences
            .entry(mutation.device_id.clone())
            .or_insert(0);
        if mutation.device_sequence > *entry {
            *entry = mutation.device_sequence;
        }
    }
}

/// Client-side helper that encrypts content and submits mutations.
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

    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    pub fn repository(&self) -> &R {
        &self.repository
    }

    pub fn repository_mut(&mut self) -> &mut R {
        &mut self.repository
    }

    pub fn crypto(&self) -> &C {
        &self.crypto
    }

    pub fn into_repository(self) -> R {
        self.repository
    }

    /// Encrypts `plaintext` and applies the resulting local mutation.
    ///
    /// Nothing but ciphertext ever reaches the repository.
    pub fn create_local_change(
        &mut self,
        entity_type: SyncEntityType,
        entity_id: Uuid,
        plaintext: &[u8],
    ) -> Result<SyncMutation, SyncError> {
        let current = self.repository.record(entity_id)?;
        if current
            .as_ref()
            .is_some_and(|record| record.metadata.tombstone)
        {
            return Err(SyncError::EntityDeleted);
        }
        let base_revision = current
            .as_ref()
            .map(|record| record.metadata.revision)
            .unwrap_or(0);
        let kind = if base_revision == 0 {
            SyncOperationKind::Create
        } else {
            SyncOperationKind::Update
        };
        let context = PayloadContext::new(entity_type.clone(), entity_id);
        let encrypted_payload = Some(self.crypto.encrypt(&context, plaintext)?);
        self.submit_local(
            entity_type,
            entity_id,
            base_revision,
            kind,
            encrypted_payload,
        )
    }

    /// Creates a local tombstone at the next revision.
    pub fn create_local_delete(
        &mut self,
        entity_type: SyncEntityType,
        entity_id: Uuid,
    ) -> Result<SyncMutation, SyncError> {
        let base_revision = self
            .repository
            .record(entity_id)?
            .map(|record| record.metadata.revision)
            .unwrap_or(0);
        self.submit_local(
            entity_type,
            entity_id,
            base_revision,
            SyncOperationKind::Delete,
            None,
        )
    }

    /// Submits a mutation whose payload is already encrypted.
    ///
    /// Used when replaying a stored version (for example accepting the incoming
    /// side of a conflict) so the ciphertext is not needlessly decrypted and
    /// re-encrypted. The payload context still binds it to `entity_id`.
    pub fn submit_encrypted(
        &mut self,
        entity_type: SyncEntityType,
        entity_id: Uuid,
        base_revision: u64,
        kind: SyncOperationKind,
        encrypted_payload: Option<EncryptedPayload>,
    ) -> Result<SyncMutation, SyncError> {
        self.submit_local(
            entity_type,
            entity_id,
            base_revision,
            kind,
            encrypted_payload,
        )
    }

    fn submit_local(
        &mut self,
        entity_type: SyncEntityType,
        entity_id: Uuid,
        base_revision: u64,
        kind: SyncOperationKind,
        encrypted_payload: Option<EncryptedPayload>,
    ) -> Result<SyncMutation, SyncError> {
        let mutation = SyncMutation {
            operation_id: Uuid::new_v4(),
            entity_id,
            entity_type,
            device_id: self.device_id.clone(),
            device_sequence: self.repository.last_device_sequence(&self.device_id)? + 1,
            base_revision,
            kind,
            timestamp: Utc::now().to_rfc3339(),
            schema_version: SyncMutation::CURRENT_SCHEMA_VERSION,
            encrypted_payload,
        };
        match self.apply_mutation(mutation.clone())? {
            ApplyOutcome::Applied { .. } => Ok(mutation),
            ApplyOutcome::AlreadyApplied { .. } => Ok(mutation),
            ApplyOutcome::Conflict { .. } => Err(SyncError::UnexpectedConflict),
            ApplyOutcome::Rejected => Err(SyncError::MutationRejected),
        }
    }

    /// Submits a mutation, whether local or received from another device.
    pub fn apply_mutation(&mut self, mutation: SyncMutation) -> Result<ApplyOutcome, SyncError> {
        self.repository.apply_mutation(mutation)
    }

    pub fn record(&self, entity_id: Uuid) -> Result<Option<SyncRecord>, SyncError> {
        self.repository.record(entity_id)
    }

    pub fn page_after(&self, cursor: SyncCursor, limit: usize) -> Result<SyncPage, SyncError> {
        self.repository.page_after(cursor, limit)
    }

    pub fn conflicts(&self) -> Result<Vec<MutationConflict>, SyncError> {
        self.repository.conflicts()
    }

    pub fn device_cursor(&self, device: &DeviceId) -> Result<SyncCursor, SyncError> {
        self.repository.device_cursor(device)
    }

    pub fn save_device_cursor(
        &mut self,
        device: &DeviceId,
        cursor: SyncCursor,
    ) -> Result<(), SyncError> {
        self.repository.save_device_cursor(device, cursor)
    }

    /// Decrypts the stored content of a record; `None` for a tombstone.
    pub fn decrypt_record(&self, record: &SyncRecord) -> Result<Option<Vec<u8>>, SyncError> {
        let payload = match record.content.as_ref() {
            Some(payload) => payload,
            None => return Ok(None),
        };
        let context = PayloadContext::new(record.metadata.entity_type.clone(), record.metadata.id);
        self.crypto.decrypt(&context, payload).map(Some)
    }

    /// Decrypts a payload that is not attached to a stored record, for example
    /// the retained incoming side of a conflict.
    pub fn decrypt_payload(
        &self,
        entity_type: &SyncEntityType,
        entity_id: Uuid,
        payload: &EncryptedPayload,
    ) -> Result<Vec<u8>, SyncError> {
        let context = PayloadContext::new(entity_type.clone(), entity_id);
        self.crypto.decrypt(&context, payload)
    }
}

/// Rejects malformed mutations before any storage work happens.
pub(crate) fn validate_mutation(mutation: &SyncMutation) -> Result<(), SyncError> {
    DeviceId::new(mutation.device_id.as_str())?;
    if mutation.device_sequence == 0 {
        return Err(SyncError::InvalidDeviceSequence);
    }
    if mutation.schema_version != SyncMutation::CURRENT_SCHEMA_VERSION {
        return Err(SyncError::UnsupportedSchemaVersion);
    }
    // The timestamp is informational only; it is validated for shape but never
    // used to order operations.
    if chrono::DateTime::parse_from_rfc3339(&mutation.timestamp).is_err() {
        return Err(SyncError::InvalidTimestamp);
    }
    // The kind is a client hint. Conflict detection is revision-based, so a
    // mislabelled stale mutation is preserved as a conflict instead of being
    // refused as malformed; only tombstone/payload consistency is enforced.
    if matches!(mutation.kind, SyncOperationKind::Delete) != mutation.encrypted_payload.is_none() {
        return Err(if matches!(mutation.kind, SyncOperationKind::Delete) {
            SyncError::InvalidTombstone
        } else {
            SyncError::MissingEncryptedPayload
        });
    }
    if mutation
        .encrypted_payload
        .as_ref()
        .is_some_and(|payload| payload.as_opaque_bytes().len() > MAX_ENCRYPTED_PAYLOAD_BYTES)
    {
        return Err(SyncError::PayloadTooLarge);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncError {
    InvalidDeviceId,
    InvalidDeviceSequence,
    InvalidTimestamp,
    InvalidTombstone,
    MissingEncryptedPayload,
    PayloadTooLarge,
    /// A local change was attempted on a deleted entity.
    EntityDeleted,
    /// The repository refused a locally created mutation; that is a bug.
    MutationRejected,
    /// A local mutation unexpectedly produced a conflict.
    UnexpectedConflict,
    /// Crypto support is missing on this platform or in this build.
    CryptoUnavailable,
    /// Decryption or backup import rejected the input.
    CryptoRejected,
    /// Malformed test-only payload.
    TestPayloadMalformed,
    /// The durable store could not be opened, read, or written.
    StorageUnavailable,
    /// The durable store is locked by another writer.
    StorageBusy,
    /// Stored rows do not match the expected schema or encoding.
    StorageCorrupt,
    /// The stored schema or payload version is newer than this build.
    UnsupportedSchemaVersion,
}

impl fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDeviceId => "invalid sync device identifier",
            Self::InvalidDeviceSequence => "invalid sync device sequence",
            Self::InvalidTimestamp => "invalid sync timestamp",
            Self::InvalidTombstone => "invalid sync tombstone",
            Self::MissingEncryptedPayload => "missing encrypted sync payload",
            Self::PayloadTooLarge => "encrypted sync payload exceeds limit",
            Self::EntityDeleted => "entity is deleted",
            Self::MutationRejected => "sync mutation was rejected by the repository",
            Self::UnexpectedConflict => "local sync mutation conflicted with stored state",
            Self::CryptoUnavailable => "sync crypto provider unavailable",
            Self::CryptoRejected => "sync crypto rejected the payload",
            Self::TestPayloadMalformed => "malformed test-only sync payload",
            Self::StorageUnavailable => "sync storage is unavailable",
            Self::StorageBusy => "sync storage is locked by another writer",
            Self::StorageCorrupt => "sync storage contains unreadable data",
            Self::UnsupportedSchemaVersion => "unsupported sync storage schema",
        })
    }
}

impl std::error::Error for SyncError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reversible fixture, not encryption. It exists only in `cfg(test)` and
    /// must never be enabled in a production configuration.
    #[derive(Default)]
    struct TestOnlyCryptoProvider;

    impl CryptoProvider for TestOnlyCryptoProvider {
        fn encrypt(
            &self,
            context: &PayloadContext,
            plaintext: &[u8],
        ) -> Result<EncryptedPayload, SyncError> {
            let mut bytes = b"test-only:".to_vec();
            bytes.extend_from_slice(context.entity_type.as_str().as_bytes());
            bytes.push(b':');
            bytes.extend(plaintext.iter().rev());
            Ok(EncryptedPayload::from_opaque_bytes(bytes))
        }

        fn decrypt(
            &self,
            context: &PayloadContext,
            payload: &EncryptedPayload,
        ) -> Result<Vec<u8>, SyncError> {
            let expected = format!("test-only:{}:", context.entity_type.as_str()).into_bytes();
            let bytes = payload
                .as_opaque_bytes()
                .strip_prefix(expected.as_slice())
                .ok_or(SyncError::TestPayloadMalformed)?;
            Ok(bytes.iter().rev().copied().collect())
        }
    }

    fn engine() -> SyncEngine<InMemorySyncRepository, TestOnlyCryptoProvider> {
        SyncEngine::new(
            InMemorySyncRepository::new(),
            TestOnlyCryptoProvider,
            DeviceId::new("desktop").unwrap(),
        )
    }

    fn mutation(
        operation_id: Uuid,
        entity_id: Uuid,
        sequence: u64,
        base_revision: u64,
        kind: SyncOperationKind,
    ) -> SyncMutation {
        SyncMutation {
            operation_id,
            entity_id,
            entity_type: SyncEntityType::Note,
            device_id: DeviceId::new("remote_device").unwrap(),
            device_sequence: sequence,
            base_revision,
            kind: kind.clone(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            schema_version: SyncMutation::CURRENT_SCHEMA_VERSION,
            encrypted_payload: if matches!(kind, SyncOperationKind::Delete) {
                None
            } else {
                Some(EncryptedPayload::from_opaque_bytes(
                    b"FICTIONAL_CIPHERTEXT".to_vec(),
                ))
            },
        }
    }

    #[test]
    fn local_change_separates_content_from_metadata() {
        let mut engine = engine();
        let entity_id = Uuid::new_v4();
        let mutation = engine
            .create_local_change(SyncEntityType::Note, entity_id, b"fictional note")
            .unwrap();

        assert_eq!(mutation.base_revision, 0);
        assert_eq!(mutation.kind, SyncOperationKind::Create);
        assert_eq!(mutation.device_sequence, 1);
        let record = engine.record(entity_id).unwrap().unwrap();
        assert_eq!(record.metadata.revision, 1);
        assert_eq!(record.metadata.device_id.as_str(), "desktop");
        let plaintext = engine.decrypt_record(&record).unwrap().unwrap();
        assert_eq!(plaintext, b"fictional note");
        assert!(!format!("{record:?}").contains("fictional note"));
    }

    #[test]
    fn sequential_local_updates_advance_the_revision() {
        let mut engine = engine();
        let entity_id = Uuid::new_v4();
        engine
            .create_local_change(SyncEntityType::Note, entity_id, b"first")
            .unwrap();
        let update = engine
            .create_local_change(SyncEntityType::Note, entity_id, b"second")
            .unwrap();

        assert_eq!(update.base_revision, 1);
        assert_eq!(update.kind, SyncOperationKind::Update);
        assert_eq!(engine.record(entity_id).unwrap().unwrap().metadata.revision, 2);
    }

    #[test]
    fn repeated_operation_is_idempotent_and_does_not_advance_the_cursor() {
        let mut repository = InMemorySyncRepository::new();
        let entity_id = Uuid::new_v4();
        let first = mutation(
            Uuid::new_v4(),
            entity_id,
            1,
            0,
            SyncOperationKind::Create,
        );
        assert_eq!(
            repository.apply_mutation(first.clone()).unwrap(),
            ApplyOutcome::Applied {
                cursor: SyncCursor(1)
            }
        );
        assert_eq!(
            repository.apply_mutation(first).unwrap(),
            ApplyOutcome::AlreadyApplied {
                cursor: SyncCursor(1)
            }
        );
        assert_eq!(
            repository.page_after(SyncCursor(1), 10).unwrap().operations.len(),
            0
        );
    }

    #[test]
    fn base_revision_mismatch_keeps_the_entity_and_stores_the_conflict() {
        let mut repository = InMemorySyncRepository::new();
        let entity_id = Uuid::new_v4();
        repository
            .apply_mutation(mutation(
                Uuid::new_v4(),
                entity_id,
                1,
                0,
                SyncOperationKind::Create,
            ))
            .unwrap();
        let stale = mutation(Uuid::new_v4(), entity_id, 2, 0, SyncOperationKind::Update);

        let outcome = repository.apply_mutation(stale.clone()).unwrap();
        let conflict_id = match outcome {
            ApplyOutcome::Conflict { conflict_id } => conflict_id,
            other => panic!("expected a conflict, got {other:?}"),
        };
        assert_eq!(
            repository.record(entity_id).unwrap().unwrap().metadata.revision,
            1
        );
        let conflicts = repository.conflicts().unwrap();
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].id, conflict_id);
        assert_eq!(conflicts[0].reason, ConflictReason::BaseRevisionMismatch);
        assert_eq!(conflicts[0].current.as_ref().unwrap().metadata.revision, 1);
        assert_eq!(conflicts[0].incoming.operation_id, stale.operation_id);

        // The same conflict is reported again without a second record.
        assert_eq!(
            repository.apply_mutation(stale).unwrap(),
            ApplyOutcome::Conflict { conflict_id }
        );
        assert_eq!(repository.conflicts().unwrap().len(), 1);
    }

    #[test]
    fn tombstones_create_a_new_revision_and_cannot_be_resurrected() {
        let mut repository = InMemorySyncRepository::new();
        let entity_id = Uuid::new_v4();
        repository
            .apply_mutation(mutation(
                Uuid::new_v4(),
                entity_id,
                1,
                0,
                SyncOperationKind::Create,
            ))
            .unwrap();
        assert_eq!(
            repository
                .apply_mutation(mutation(
                    Uuid::new_v4(),
                    entity_id,
                    2,
                    1,
                    SyncOperationKind::Delete
                ))
                .unwrap(),
            ApplyOutcome::Applied {
                cursor: SyncCursor(2)
            }
        );
        let record = repository.record(entity_id).unwrap().unwrap();
        assert!(record.metadata.tombstone);
        assert!(record.content.is_none());
        assert_eq!(record.metadata.revision, 2);

        // A later update at the correct base revision is still refused.
        assert_eq!(
            repository
                .apply_mutation(mutation(
                    Uuid::new_v4(),
                    entity_id,
                    3,
                    2,
                    SyncOperationKind::Update
                ))
                .unwrap(),
            ApplyOutcome::Rejected
        );
        assert_eq!(
            repository.record(entity_id).unwrap().unwrap().metadata.revision,
            2
        );
    }

    #[test]
    fn duplicate_device_sequence_with_a_new_operation_is_rejected() {
        let mut repository = InMemorySyncRepository::new();
        repository
            .apply_mutation(mutation(
                Uuid::new_v4(),
                Uuid::new_v4(),
                1,
                0,
                SyncOperationKind::Create,
            ))
            .unwrap();
        assert_eq!(
            repository
                .apply_mutation(mutation(
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    1,
                    0,
                    SyncOperationKind::Create
                ))
                .unwrap(),
            ApplyOutcome::Rejected
        );
        assert_eq!(repository.page_after(SyncCursor(0), 10).unwrap().operations.len(), 1);
    }

    #[test]
    fn cursors_skip_conflicts_and_respect_the_page_limit() {
        let mut repository = InMemorySyncRepository::new();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        repository
            .apply_mutation(mutation(
                Uuid::new_v4(),
                first,
                1,
                0,
                SyncOperationKind::Create,
            ))
            .unwrap();
        repository
            .apply_mutation(mutation(
                Uuid::new_v4(),
                second,
                2,
                0,
                SyncOperationKind::Create,
            ))
            .unwrap();
        // Sequence 3 is a conflict and must consume a cursor without being replicated.
        repository
            .apply_mutation(mutation(
                Uuid::new_v4(),
                first,
                3,
                0,
                SyncOperationKind::Update,
            ))
            .unwrap();

        let page = repository.page_after(SyncCursor(0), 1).unwrap();
        assert_eq!(page.operations.len(), 1);
        assert_eq!(page.next_cursor, SyncCursor(1));
        let rest = repository.page_after(page.next_cursor, 10).unwrap();
        assert_eq!(rest.operations.len(), 1);
        assert_eq!(rest.next_cursor, SyncCursor(3));
        assert_eq!(rest.operations[0].mutation.entity_id, second);
    }

    #[test]
    fn device_sequences_continue_after_a_restart_of_the_engine() {
        let mut repository = InMemorySyncRepository::new();
        let entity_id = Uuid::new_v4();
        let first = mutation(
            Uuid::new_v4(),
            entity_id,
            7,
            0,
            SyncOperationKind::Create,
        );
        repository.apply_mutation(first).unwrap();
        assert_eq!(
            repository
                .last_device_sequence(&DeviceId::new("remote_device").unwrap())
                .unwrap(),
            7
        );
    }

    #[test]
    fn device_cursors_round_trip() {
        let mut repository = InMemorySyncRepository::new();
        let device = DeviceId::new("remote_device").unwrap();
        assert_eq!(repository.device_cursor(&device).unwrap(), SyncCursor(0));
        repository
            .save_device_cursor(&device, SyncCursor(9))
            .unwrap();
        assert_eq!(repository.device_cursor(&device).unwrap(), SyncCursor(9));
    }

    #[test]
    fn malformed_mutations_are_rejected_before_storage() {
        let mut repository = InMemorySyncRepository::new();
        let bad_sequence = mutation(
            Uuid::new_v4(),
            Uuid::new_v4(),
            0,
            0,
            SyncOperationKind::Create,
        );
        assert_eq!(
            repository.apply_mutation(bad_sequence),
            Err(SyncError::InvalidDeviceSequence)
        );

        let mut bad_timestamp = mutation(
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            0,
            SyncOperationKind::Create,
        );
        bad_timestamp.timestamp = "not-a-timestamp".into();
        assert_eq!(
            repository.apply_mutation(bad_timestamp),
            Err(SyncError::InvalidTimestamp)
        );

        let mut missing_payload = mutation(
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            0,
            SyncOperationKind::Create,
        );
        missing_payload.encrypted_payload = None;
        assert_eq!(
            repository.apply_mutation(missing_payload),
            Err(SyncError::MissingEncryptedPayload)
        );

        assert!(repository
            .page_after(SyncCursor(0), 10)
            .unwrap()
            .operations
            .is_empty());
    }

    #[test]
    fn a_mislabelled_stale_mutation_is_a_conflict_not_a_validation_error() {
        let mut repository = InMemorySyncRepository::new();
        let entity_id = Uuid::new_v4();
        repository
            .apply_mutation(mutation(
                Uuid::new_v4(),
                entity_id,
                1,
                0,
                SyncOperationKind::Create,
            ))
            .unwrap();

        // A stale client that still believes the entity does not exist must be
        // preserved as a conflict even though it labels the change an update.
        let mislabelled = mutation(Uuid::new_v4(), entity_id, 2, 0, SyncOperationKind::Update);
        assert!(matches!(
            repository.apply_mutation(mislabelled).unwrap(),
            ApplyOutcome::Conflict { .. }
        ));
        assert_eq!(
            repository.record(entity_id).unwrap().unwrap().metadata.revision,
            1
        );
    }

    #[test]
    fn local_changes_on_a_tombstone_are_refused() {
        let mut engine = engine();
        let entity_id = Uuid::new_v4();
        engine
            .create_local_change(SyncEntityType::Note, entity_id, b"fixture")
            .unwrap();
        engine
            .create_local_delete(SyncEntityType::Note, entity_id)
            .unwrap();
        assert_eq!(
            engine.create_local_change(SyncEntityType::Note, entity_id, b"resurrect"),
            Err(SyncError::EntityDeleted)
        );
    }

    #[test]
    fn debug_output_and_diagnostics_never_include_entity_content() {
        let mut engine = engine();
        let secret_note = "FICTIONAL_NOTE_CONTENT_DO_NOT_LOG";
        let fake_password = "FICTIONAL_PASSWORD_DO_NOT_LOG";
        let entity_id = Uuid::new_v4();
        let mutation = engine
            .create_local_change(
                SyncEntityType::VaultRecord,
                entity_id,
                format!("{secret_note}:{fake_password}").as_bytes(),
            )
            .unwrap();

        let record = engine.record(entity_id).unwrap().unwrap();
        let stored = engine.page_after(SyncCursor(0), 10).unwrap();
        let rendered = format!("{mutation:?}{record:?}{stored:?}");
        assert!(!rendered.contains(secret_note));
        assert!(!rendered.contains(fake_password));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn payload_context_is_bound_to_the_entity() {
        let crypto = TestOnlyCryptoProvider;
        let entity_id = Uuid::new_v4();
        let context = PayloadContext::new(SyncEntityType::Note, entity_id);
        let payload = crypto.encrypt(&context, b"fixture").unwrap();
        assert_eq!(crypto.decrypt(&context, &payload).unwrap(), b"fixture");

        let other = PayloadContext::new(SyncEntityType::VaultRecord, entity_id);
        assert_eq!(
            crypto.decrypt(&other, &payload),
            Err(SyncError::TestPayloadMalformed)
        );
    }
}
