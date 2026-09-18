//! `VaultStore`: encrypted password storage on top of the synchronization layer.
//!
//! It uses the same [`SyncRepository`], `SyncEngine`, `SqliteSyncRepository`,
//! production crypto, DPAPI protection, and portable backup as the notes
//! feature, but:
//!
//! * it lives in its own database file, so journals and cursors never mix;
//! * it uses [`SyncEntityType::VaultRecord`];
//! * its payload cipher is a [`PurposeKeyProvider`] for `JARVIS/vault/v1`, which
//!   holds a key derived from the master key and never the master key itself.
//!
//! Reads decrypt the whole item set once into memory (that is what makes search
//! work without any plaintext in SQLite) and every write invalidates the cache.
//! Secrets live in `VaultItemPayload`, which zeroizes on drop.

use crate::sync::crypto::PurposeKeyProvider;
use crate::sync::sqlite::SqliteSyncRepository;
use crate::sync::{
    CryptoProvider, DeviceId, MutationConflict, SyncCursor, SyncEngine, SyncEntityType,
    SyncOperationKind, SyncRecord, SyncRepository,
};
use std::collections::BTreeSet;
use uuid::Uuid;

use super::model::*;

/// Concrete store used by the application: the vault database with the vault key.
pub type EncryptedVaultStore = VaultStore<SqliteSyncRepository, PurposeKeyProvider>;

/// How long a revealed secret may stay on screen by default.
pub const DEFAULT_REVEAL_TIMEOUT_SECONDS: u64 = 30;

/// Result of reading one entity from storage.
enum EntityRead<T> {
    Missing,
    Tombstoned,
    Present(T),
    Unreadable,
}

/// Decrypted view of the vault: identifiers, payloads, and revisions in step.
#[derive(Default)]
struct VaultCache {
    loaded: bool,
    ids: Vec<Uuid>,
    revisions: Vec<u64>,
    items: Vec<VaultItemPayload>,
    /// Items that exist but could not be decrypted; reported, never hidden.
    unreadable: usize,
}

impl VaultCache {
    fn upsert(&mut self, id: Uuid, revision: u64, payload: VaultItemPayload) {
        if !self.loaded {
            return;
        }
        match self.ids.iter().position(|known| *known == id) {
            Some(index) => {
                self.items[index] = payload;
                self.revisions[index] = revision;
            }
            None => {
                self.ids.push(id);
                self.items.push(payload);
                self.revisions.push(revision);
            }
        }
    }
}

/// Encrypted password storage.
pub struct VaultStore<R: SyncRepository, C: CryptoProvider> {
    engine: SyncEngine<R, C>,
    cache: VaultCache,
}

impl<R: SyncRepository, C: CryptoProvider> VaultStore<R, C> {
    pub fn new(repository: R, crypto: C, device_id: DeviceId) -> Self {
        Self {
            engine: SyncEngine::new(repository, crypto, device_id),
            cache: VaultCache::default(),
        }
    }

    pub fn device_id(&self) -> &DeviceId {
        self.engine.device_id()
    }

    pub fn repository(&self) -> &R {
        self.engine.repository()
    }

    pub fn into_repository(self) -> R {
        self.engine.into_repository()
    }

    /// Mutable repository access, for example to submit a mutation that did not
    /// originate from this store or to advance a device cursor.
    pub fn repository_mut(&mut self) -> &mut R {
        self.engine.repository_mut()
    }

    /// Drops the decrypted cache; the next read reloads and re-decrypts.
    pub fn invalidate(&mut self) {
        self.cache = VaultCache::default();
    }

    /// Whether decrypted items are currently held in memory.
    pub fn holds_decrypted_items(&self) -> bool {
        self.cache.loaded && !self.cache.items.is_empty()
    }

    pub fn has_stored_entities(&self) -> Result<bool, VaultError> {
        let page = self.engine.page_after(SyncCursor(0), 1)?;
        Ok(!page.operations.is_empty())
    }

    // ------------------------------------------------------------------- CRUD

    pub fn create_item(&mut self, draft: &VaultItemDraft) -> Result<VaultItemDetails, VaultError> {
        let payload = VaultItemPayload::create(draft)?;
        self.write_item(Uuid::new_v4(), &payload)
    }

    pub fn update_item(
        &mut self,
        id: Uuid,
        draft: &VaultItemDraft,
    ) -> Result<VaultItemDetails, VaultError> {
        let mut payload = self.require_payload(id)?;
        payload.apply_draft(draft)?;
        self.write_item(id, &payload)
    }

    /// Edits only the non-secret fields.
    ///
    /// This is what the interface uses when the user never revealed the secret:
    /// renaming or retagging an item must not be able to erase its password.
    pub fn update_metadata(
        &mut self,
        id: Uuid,
        metadata: &VaultMetadataDraft,
    ) -> Result<VaultItemDetails, VaultError> {
        let mut payload = self.require_payload(id)?;
        payload.apply_metadata(metadata)?;
        self.write_item(id, &payload)
    }

    /// Replaces the secret fields after an explicit reveal.
    pub fn update_secrets(
        &mut self,
        id: Uuid,
        password: &str,
        notes: &str,
    ) -> Result<VaultItemDetails, VaultError> {
        let mut payload = self.require_payload(id)?;
        payload.apply_secrets(password, notes)?;
        self.write_item(id, &payload)
    }

    /// Item details without the password and without the free-form notes.
    pub fn get_item(&mut self, id: Uuid) -> Result<Option<VaultItemDetails>, VaultError> {
        match self.read_item(id)? {
            EntityRead::Present((payload, revision)) => Ok(Some(payload.to_details(id, revision))),
            EntityRead::Missing | EntityRead::Tombstoned => Ok(None),
            EntityRead::Unreadable => Err(VaultError::Unreadable),
        }
    }

    /// Explicit secret request: the only path that returns a password.
    pub fn reveal_secret(
        &mut self,
        id: Uuid,
        reveal_timeout_seconds: u64,
    ) -> Result<SecretRevealResult, VaultError> {
        match self.read_item(id)? {
            EntityRead::Present((payload, revision)) => Ok(SecretRevealResult {
                id,
                revision,
                password: payload.password.clone(),
                notes: payload.notes.clone(),
                reveal_timeout_seconds: reveal_timeout_seconds.clamp(5, 300),
            }),
            EntityRead::Missing => Err(VaultError::NotFound),
            EntityRead::Tombstoned => Err(VaultError::Deleted),
            EntityRead::Unreadable => Err(VaultError::Unreadable),
        }
    }

    /// Password only, for Rust-side clipboard copy: never crosses the IPC bridge.
    pub fn password_for_clipboard(&mut self, id: Uuid) -> Result<String, VaultError> {
        match self.read_item(id)? {
            EntityRead::Present((payload, _)) if !payload.password.is_empty() => {
                Ok(payload.password.clone())
            }
            EntityRead::Present(_) => Err(VaultError::NotFound),
            EntityRead::Missing => Err(VaultError::NotFound),
            EntityRead::Tombstoned => Err(VaultError::Deleted),
            EntityRead::Unreadable => Err(VaultError::Unreadable),
        }
    }

    /// Username only, for Rust-side clipboard copy.
    pub fn username_for_clipboard(&mut self, id: Uuid) -> Result<String, VaultError> {
        match self.read_item(id)? {
            EntityRead::Present((payload, _)) if !payload.username.is_empty() => {
                Ok(payload.username.clone())
            }
            EntityRead::Present(_) => Err(VaultError::NotFound),
            EntityRead::Missing => Err(VaultError::NotFound),
            EntityRead::Tombstoned => Err(VaultError::Deleted),
            EntityRead::Unreadable => Err(VaultError::Unreadable),
        }
    }

    pub fn set_favorite(
        &mut self,
        id: Uuid,
        favorite: bool,
    ) -> Result<VaultItemDetails, VaultError> {
        let mut payload = self.require_payload(id)?;
        if payload.favorite != favorite {
            payload.favorite = favorite;
            payload.touch();
        }
        self.write_item(id, &payload)
    }

    pub fn trash_item(&mut self, id: Uuid) -> Result<VaultItemDetails, VaultError> {
        let mut payload = self.require_payload(id)?;
        if payload.deleted_at.is_none() {
            payload.deleted_at = Some(chrono::Utc::now().to_rfc3339());
            payload.touch();
        }
        self.write_item(id, &payload)
    }

    pub fn restore_item(&mut self, id: Uuid) -> Result<VaultItemDetails, VaultError> {
        let mut payload = self.require_payload(id)?;
        if payload.deleted_at.is_some() {
            payload.deleted_at = None;
            payload.touch();
        }
        self.write_item(id, &payload)
    }

    /// Permanent deletion: writes a tombstone, which drops the payload.
    pub fn purge_item(&mut self, id: Uuid) -> Result<(), VaultError> {
        self.engine
            .create_local_delete(SyncEntityType::VaultRecord, id)?;
        self.invalidate();
        Ok(())
    }

    // ------------------------------------------------------------------- list

    pub fn list_items(&mut self, query: &VaultQuery) -> Result<VaultItemList, VaultError> {
        self.ensure_loaded()?;
        let indices = matching_indices(&self.cache.items, query);
        let items = indices
            .iter()
            .map(|index| {
                self.cache.items[*index]
                    .to_summary(self.cache.ids[*index], self.cache.revisions[*index])
            })
            .collect();
        Ok(VaultItemList {
            items,
            unreadable: self.cache.unreadable,
            scanned: self.cache.items.len(),
        })
    }

    pub fn all_tags(&mut self) -> Result<Vec<String>, VaultError> {
        self.ensure_loaded()?;
        let active: Vec<VaultItemPayload> = self
            .cache
            .items
            .iter()
            .filter(|item| !item.is_trashed())
            .cloned()
            .collect();
        Ok(collect_tags(&active))
    }

    pub fn stats(&mut self) -> Result<VaultStats, VaultError> {
        self.ensure_loaded()?;
        Ok(VaultStats {
            items_total: self
                .cache
                .items
                .iter()
                .filter(|item| !item.is_trashed())
                .count(),
            items_trashed: self
                .cache
                .items
                .iter()
                .filter(|item| item.is_trashed())
                .count(),
            favorites: self
                .cache
                .items
                .iter()
                .filter(|item| item.favorite && !item.is_trashed())
                .count(),
            conflicts_pending: self.engine.conflicts()?.len(),
            unreadable: self.cache.unreadable,
        })
    }

    pub fn pending_conflict_count(&self) -> Result<usize, VaultError> {
        Ok(self.engine.conflicts()?.len())
    }

    // -------------------------------------------------------------- conflicts

    pub fn conflicts(&self) -> Result<Vec<VaultConflictView>, VaultError> {
        let conflicts = self.engine.conflicts()?;
        Ok(conflicts
            .iter()
            .map(|conflict| self.conflict_view(conflict))
            .collect())
    }

    /// Resolves a retained conflict without destroying either version.
    pub fn resolve_conflict(
        &mut self,
        conflict_id: Uuid,
        resolution: VaultConflictResolution,
    ) -> Result<VaultConflictOutcome, VaultError> {
        let conflict = self
            .engine
            .conflicts()?
            .into_iter()
            .find(|conflict| conflict.id == conflict_id)
            .ok_or(VaultError::NotFound)?;
        let entity_id = conflict.entity_id;

        let mut created_entity_id = None;
        let mut updated_entity_id = None;
        match resolution {
            VaultConflictResolution::KeepCurrent => {}
            VaultConflictResolution::AcceptIncoming => {
                let current = self.engine.record(entity_id)?;
                if current
                    .as_ref()
                    .is_some_and(|record| record.metadata.tombstone)
                {
                    // A tombstone cannot be overwritten; keeping both is the only
                    // lossless option.
                    return Err(VaultError::Deleted);
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
                let payload = conflict
                    .incoming
                    .encrypted_payload
                    .clone()
                    .ok_or(VaultError::Unreadable)?;
                self.engine.submit_encrypted(
                    SyncEntityType::VaultRecord,
                    entity_id,
                    base_revision,
                    kind,
                    Some(payload),
                )?;
                updated_entity_id = Some(entity_id);
            }
            VaultConflictResolution::KeepBoth => {
                let payload = conflict
                    .incoming
                    .encrypted_payload
                    .as_ref()
                    .ok_or(VaultError::Unreadable)?;
                let bytes =
                    self.engine
                        .decrypt_payload(&SyncEntityType::VaultRecord, entity_id, payload)?;
                let parsed = VaultItemPayload::from_bytes(&bytes)?;
                created_entity_id = Some(self.write_item(Uuid::new_v4(), &parsed)?.id);
            }
        }

        if !self.engine.repository_mut().discard_conflict(conflict_id)? {
            return Err(VaultError::NotFound);
        }
        self.invalidate();
        Ok(VaultConflictOutcome {
            resolution,
            created_entity_id,
            updated_entity_id,
        })
    }

    // ------------------------------------------------------------- internals

    fn conflict_view(&self, conflict: &MutationConflict) -> VaultConflictView {
        VaultConflictView {
            conflict_id: conflict.id,
            entity_id: conflict.entity_id,
            is_vault_item: matches!(conflict.entity_type, SyncEntityType::VaultRecord),
            current_name: self
                .payload_from_record(conflict.current.as_ref())
                .map(|payload| payload.name.clone()),
            incoming_name: if matches!(conflict.entity_type, SyncEntityType::VaultRecord) {
                self.incoming_payload(conflict)
                    .map(|payload| payload.name.clone())
            } else {
                None
            },
            current_revision: conflict.entity_revision,
            incoming_revision: conflict.incoming.base_revision + 1,
        }
    }

    fn incoming_payload(&self, conflict: &MutationConflict) -> Option<VaultItemPayload> {
        let payload = conflict.incoming.encrypted_payload.as_ref()?;
        let bytes = self
            .engine
            .decrypt_payload(&conflict.entity_type, conflict.entity_id, payload)
            .ok()?;
        VaultItemPayload::from_bytes(&bytes).ok()
    }

    fn payload_from_record(&self, record: Option<&SyncRecord>) -> Option<VaultItemPayload> {
        let record = record?;
        if record.metadata.tombstone
            || !matches!(record.metadata.entity_type, SyncEntityType::VaultRecord)
        {
            return None;
        }
        let bytes = self.engine.decrypt_record(record).ok()??;
        VaultItemPayload::from_bytes(&bytes).ok()
    }

    fn require_payload(&mut self, id: Uuid) -> Result<VaultItemPayload, VaultError> {
        match self.read_item(id)? {
            EntityRead::Present((payload, _)) => Ok(payload),
            EntityRead::Missing => Err(VaultError::NotFound),
            EntityRead::Tombstoned => Err(VaultError::Deleted),
            EntityRead::Unreadable => Err(VaultError::Unreadable),
        }
    }

    fn write_item(
        &mut self,
        id: Uuid,
        payload: &VaultItemPayload,
    ) -> Result<VaultItemDetails, VaultError> {
        payload.validate()?;
        let bytes = payload.to_bytes()?;
        let mutation = self
            .engine
            .create_local_change(SyncEntityType::VaultRecord, id, &bytes)?;
        let revision = match self.engine.record(id)? {
            Some(record) => record.metadata.revision,
            None => mutation.base_revision + 1,
        };
        self.cache.upsert(id, revision, payload.clone());
        Ok(payload.to_details(id, revision))
    }

    fn read_item(&self, id: Uuid) -> Result<EntityRead<(VaultItemPayload, u64)>, VaultError> {
        let record = match self.engine.record(id)? {
            Some(record) => record,
            None => return Ok(EntityRead::Missing),
        };
        if record.metadata.tombstone {
            return Ok(EntityRead::Tombstoned);
        }
        if !matches!(record.metadata.entity_type, SyncEntityType::VaultRecord) {
            return Ok(EntityRead::Missing);
        }
        let bytes = match self.engine.decrypt_record(&record) {
            Ok(Some(bytes)) => bytes,
            // A record this domain cannot decrypt (wrong key, tampered, or an
            // unexpected version) is reported as unreadable instead of failing
            // the whole listing.
            Ok(None) | Err(_) => return Ok(EntityRead::Unreadable),
        };
        match VaultItemPayload::from_bytes(&bytes) {
            Ok(payload) => Ok(EntityRead::Present((payload, record.metadata.revision))),
            Err(_) => Ok(EntityRead::Unreadable),
        }
    }

    fn ensure_loaded(&mut self) -> Result<(), VaultError> {
        if self.cache.loaded {
            return Ok(());
        }
        let ids = self.entity_ids()?;
        let mut cache = VaultCache {
            loaded: true,
            ids: Vec::with_capacity(ids.len()),
            revisions: Vec::with_capacity(ids.len()),
            items: Vec::with_capacity(ids.len()),
            unreadable: 0,
        };
        for id in ids {
            match self.read_item(id)? {
                EntityRead::Present((payload, revision)) => {
                    cache.ids.push(id);
                    cache.revisions.push(revision);
                    cache.items.push(payload);
                }
                EntityRead::Unreadable => cache.unreadable += 1,
                EntityRead::Missing | EntityRead::Tombstoned => {}
            }
        }
        self.cache = cache;
        Ok(())
    }

    /// Every vault entity identifier that ever appeared in the applied journal.
    fn entity_ids(&self) -> Result<Vec<Uuid>, VaultError> {
        let mut cursor = SyncCursor(0);
        let mut collected: BTreeSet<Uuid> = BTreeSet::new();
        loop {
            let page = self.engine.page_after(cursor, crate::sync::MAX_PAGE_SIZE)?;
            if page.operations.is_empty() {
                break;
            }
            for operation in &page.operations {
                if matches!(operation.mutation.entity_type, SyncEntityType::VaultRecord) {
                    collected.insert(operation.mutation.entity_id);
                }
            }
            if page.next_cursor <= cursor {
                break;
            }
            cursor = page.next_cursor;
        }
        Ok(collected.into_iter().collect())
    }
}

/// Decrypted payloads, for tests that must inspect stored content.
#[cfg(test)]
impl<R: SyncRepository, C: CryptoProvider> VaultStore<R, C> {
    pub(crate) fn decrypt_item_set(&mut self) -> Result<Vec<VaultItemPayload>, VaultError> {
        self.ensure_loaded()?;
        Ok(self.cache.items.clone())
    }
}
