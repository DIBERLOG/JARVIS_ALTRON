//! `NoteStore`: CRUD over encrypted note and folder entities.
//!
//! Every write goes through [`SyncEngine`], so nothing but ciphertext reaches
//! the repository and every change becomes a new revision. Reads decrypt the
//! whole note set once and keep it in memory, which is what makes search,
//! sorting, and tag filters possible without any plaintext in SQLite.
//!
//! Performance note: the first read after a restart walks the journal and
//! decrypts every entity once. Later reads reuse the in-memory cache, which is
//! invalidated on every write. Memory use is therefore proportional to the total
//! size of all decrypted notes; see `NOTES.md`.

use crate::sync::sqlite::SqliteSyncRepository;
use crate::sync::crypto::MasterKeyCryptoProvider;
use crate::sync::{
    CryptoProvider, DeviceId, MutationConflict, SyncCursor, SyncEngine, SyncEntityType,
    SyncOperationKind, SyncRecord, SyncRepository,
};
use std::collections::BTreeSet;
use uuid::Uuid;

use super::model::*;

/// Concrete store used by the application.
pub type EncryptedNoteStore = NoteStore<SqliteSyncRepository, MasterKeyCryptoProvider>;

/// Result of reading one entity from storage.
enum EntityRead<T> {
    /// No record exists for this identifier.
    Missing,
    /// The record is a tombstone; the content is gone on purpose.
    Tombstoned,
    /// A readable version.
    Present(T),
    /// A record exists but its payload cannot be decrypted or parsed.
    Unreadable,
}

#[derive(Default)]
struct NoteCache {
    loaded: bool,
    notes: Vec<Note>,
    folders: Vec<NoteFolder>,
    /// Notes whose payload could not be read; reported, never hidden.
    unreadable: usize,
}

impl NoteCache {
    fn upsert_note(&mut self, note: Note) {
        match self.notes.iter_mut().find(|stored| stored.id == note.id) {
            Some(stored) => *stored = note,
            None => self.notes.push(note),
        }
    }

    fn remove_note(&mut self, id: Uuid) {
        self.notes.retain(|note| note.id != id);
    }

    fn upsert_folder(&mut self, folder: NoteFolder) {
        match self.folders.iter_mut().find(|stored| stored.id == folder.id) {
            Some(stored) => *stored = folder,
            None => self.folders.push(folder),
        }
    }

    fn remove_folder(&mut self, id: Uuid) {
        self.folders.retain(|folder| folder.id != id);
    }
}

/// Encrypted note and folder storage on top of a [`SyncRepository`].
pub struct NoteStore<R: SyncRepository, C: CryptoProvider> {
    engine: SyncEngine<R, C>,
    cache: NoteCache,
}

impl<R: SyncRepository, C: CryptoProvider> NoteStore<R, C> {
    pub fn new(repository: R, crypto: C, device_id: DeviceId) -> Self {
        Self {
            engine: SyncEngine::new(repository, crypto, device_id),
            cache: NoteCache::default(),
        }
    }

    pub fn device_id(&self) -> &DeviceId {
        self.engine.device_id()
    }

    pub fn repository(&self) -> &R {
        self.engine.repository()
    }

    /// Borrows the payload cipher; used by the vault to re-wrap the master key.
    pub fn crypto(&self) -> &C {
        self.engine.crypto()
    }

    /// Mutable repository access, for example to advance a device cursor or to
    /// submit a mutation that did not originate from this store.
    pub fn repository_mut(&mut self) -> &mut R {
        self.engine.repository_mut()
    }

    /// Gives the repository back, for example to relock the storage.
    pub fn into_repository(self) -> R {
        self.engine.into_repository()
    }

    /// Drops the in-memory cache; the next read reloads and re-decrypts.
    pub fn invalidate(&mut self) {
        self.cache = NoteCache::default();
    }

    // ----------------------------------------------------------------- notes

    pub fn create_note(&mut self, draft: &NoteDraft) -> Result<Note, NoteError> {
        let payload = NotePayload::create(draft)?;
        self.write_note(Uuid::new_v4(), &payload)
    }

    pub fn update_note(&mut self, id: Uuid, draft: &NoteDraft) -> Result<Note, NoteError> {
        let mut payload = self.require_payload(id)?;
        payload.apply_draft(draft)?;
        self.write_note(id, &payload)
    }

    pub fn set_pinned(&mut self, id: Uuid, pinned: bool) -> Result<Note, NoteError> {
        let mut payload = self.require_payload(id)?;
        if payload.pinned != pinned {
            payload.pinned = pinned;
            payload.touch();
        }
        self.write_note(id, &payload)
    }

    /// Moves a note to the trash. The content is kept.
    pub fn trash_note(&mut self, id: Uuid) -> Result<Note, NoteError> {
        let mut payload = self.require_payload(id)?;
        if payload.deleted_at.is_none() {
            payload.deleted_at = Some(chrono::Utc::now().to_rfc3339());
            payload.touch();
        }
        self.write_note(id, &payload)
    }

    pub fn restore_note(&mut self, id: Uuid) -> Result<Note, NoteError> {
        let mut payload = self.require_payload(id)?;
        if payload.deleted_at.is_some() {
            payload.deleted_at = None;
            payload.touch();
        }
        self.write_note(id, &payload)
    }

    /// Permanently removes a note by writing a tombstone.
    ///
    /// Afterwards the payload is gone from storage; the journal keeps only the
    /// fact that the entity existed.
    pub fn purge_note(&mut self, id: Uuid) -> Result<(), NoteError> {
        self.engine.create_local_delete(SyncEntityType::Note, id)?;
        self.cache.remove_note(id);
        Ok(())
    }

    pub fn get_note(&mut self, id: Uuid) -> Result<Option<Note>, NoteError> {
        match self.read_note(id)? {
            EntityRead::Present(note) => Ok(Some(note)),
            EntityRead::Missing | EntityRead::Tombstoned => Ok(None),
            EntityRead::Unreadable => Err(NoteError::Unreadable),
        }
    }

    pub fn list_notes(&mut self, query: &NoteQuery) -> Result<NoteList, NoteError> {
        self.ensure_loaded()?;
        let indices = matching_indices(&self.cache.notes, query);
        let items = indices
            .iter()
            .map(|index| self.cache.notes[*index].to_summary())
            .collect();
        Ok(NoteList {
            items,
            unreadable: self.cache.unreadable,
            scanned: self.cache.notes.len(),
        })
    }

    /// Decrypted copies of every live note, for tests and export paths.
    pub fn decrypt_note_set(&mut self) -> Result<Vec<Note>, NoteError> {
        self.ensure_loaded()?;
        Ok(self.cache.notes.clone())
    }

    pub fn all_tags(&mut self) -> Result<Vec<String>, NoteError> {
        self.ensure_loaded()?;
        let active: Vec<Note> = self
            .cache
            .notes
            .iter()
            .filter(|note| !note.is_trashed())
            .cloned()
            .collect();
        Ok(collect_tags(&active))
    }

    // --------------------------------------------------------------- folders

    pub fn create_folder(&mut self, name: &str) -> Result<NoteFolder, NoteError> {
        let payload = FolderPayload::create(name)?;
        self.write_folder(Uuid::new_v4(), &payload)
    }

    pub fn rename_folder(&mut self, id: Uuid, name: &str) -> Result<NoteFolder, NoteError> {
        let mut payload = self.require_folder_payload(id)?;
        payload.rename(name)?;
        self.write_folder(id, &payload)
    }

    pub fn trash_folder(&mut self, id: Uuid) -> Result<NoteFolder, NoteError> {
        let mut payload = self.require_folder_payload(id)?;
        if payload.deleted_at.is_none() {
            payload.deleted_at = Some(chrono::Utc::now().to_rfc3339());
            payload.updated_at = chrono::Utc::now().to_rfc3339();
        }
        self.write_folder(id, &payload)
    }

    pub fn restore_folder(&mut self, id: Uuid) -> Result<NoteFolder, NoteError> {
        let mut payload = self.require_folder_payload(id)?;
        if payload.deleted_at.is_some() {
            payload.deleted_at = None;
            payload.updated_at = chrono::Utc::now().to_rfc3339();
        }
        self.write_folder(id, &payload)
    }

    /// Permanently removes a folder and detaches the notes that referenced it.
    pub fn purge_folder(&mut self, id: Uuid) -> Result<(), NoteError> {
        self.ensure_loaded()?;
        let affected: Vec<Uuid> = self
            .cache
            .notes
            .iter()
            .filter(|note| note.folder_id == Some(id))
            .map(|note| note.id)
            .collect();
        for note_id in affected {
            // Detach without touching `updated_at`: the note itself did not change.
            let mut payload = self.require_payload(note_id)?;
            payload.folder_id = None;
            self.write_note(note_id, &payload)?;
        }
        self.engine
            .create_local_delete(SyncEntityType::NoteFolder, id)?;
        self.cache.remove_folder(id);
        Ok(())
    }

    pub fn folders(&mut self) -> Result<Vec<NoteFolder>, NoteError> {
        self.ensure_loaded()?;
        let mut folders = self.cache.folders.clone();
        folders.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(folders)
    }

    // ------------------------------------------------------------- conflicts

    pub fn conflicts(&self) -> Result<Vec<NoteConflictView>, NoteError> {
        let conflicts = self.engine.conflicts()?;
        Ok(conflicts
            .iter()
            .map(|conflict| self.conflict_view(conflict))
            .collect())
    }

    pub fn stats(&mut self) -> Result<NoteStats, NoteError> {
        self.ensure_loaded()?;
        Ok(NoteStats {
            notes_total: self.cache.notes.iter().filter(|note| !note.is_trashed()).count(),
            notes_trashed: self.cache.notes.iter().filter(|note| note.is_trashed()).count(),
            folders_total: self.cache.folders.iter().filter(|folder| !folder.is_trashed()).count(),
            conflicts_pending: self.engine.conflicts()?.len(),
            unreadable: self.cache.unreadable,
        })
    }

    /// Number of pending conflicts; does not require the cache to be loaded.
    pub fn pending_conflict_count(&self) -> Result<usize, NoteError> {
        Ok(self.engine.conflicts()?.len())
    }

    /// Whether storage already holds applied operations, without decrypting.
    pub fn has_stored_entities(&self) -> Result<bool, NoteError> {
        let page = self.engine.page_after(SyncCursor(0), 1)?;
        Ok(!page.operations.is_empty())
    }

    /// Resolves a retained conflict.
    ///
    /// No version disappears implicitly: each choice is an explicit operator
    /// action, and the journal keeps the incoming mutation either way.
    pub fn resolve_conflict(
        &mut self,
        conflict_id: Uuid,
        resolution: NoteConflictResolution,
    ) -> Result<ConflictResolutionOutcome, NoteError> {
        let conflict = self
            .engine
            .conflicts()?
            .into_iter()
            .find(|conflict| conflict.id == conflict_id)
            .ok_or(NoteError::NotFound)?;
        let entity_type = conflict.entity_type.clone();
        let entity_id = conflict.entity_id;

        let mut created_entity_id = None;
        let mut updated_entity_id = None;
        match resolution {
            NoteConflictResolution::KeepCurrent => {}
            NoteConflictResolution::AcceptIncoming => {
                let current = self.engine.record(entity_id)?;
                if current
                    .as_ref()
                    .is_some_and(|record| record.metadata.tombstone)
                {
                    // A tombstone cannot be overwritten; keeping both versions is
                    // the only lossless option.
                    return Err(NoteError::Deleted);
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
                // Reuse the retained ciphertext: its associated data is bound to
                // this same entity type and identifier.
                let payload = conflict
                    .incoming
                    .encrypted_payload
                    .clone()
                    .ok_or(NoteError::ConflictUnreadable)?;
                self.engine.submit_encrypted(
                    entity_type.clone(),
                    entity_id,
                    base_revision,
                    kind,
                    Some(payload),
                )?;
                updated_entity_id = Some(entity_id);
            }
            NoteConflictResolution::KeepBoth => {
                created_entity_id = self.copy_incoming_version(&conflict)?;
            }
        }

        if !self
            .engine
            .repository_mut()
            .discard_conflict(conflict_id)?
        {
            return Err(NoteError::NotFound);
        }
        self.invalidate();
        Ok(ConflictResolutionOutcome {
            resolution,
            created_entity_id,
            updated_entity_id,
        })
    }

    // ------------------------------------------------------------ internals

    fn conflict_view(&self, conflict: &MutationConflict) -> NoteConflictView {
        NoteConflictView {
            conflict_id: conflict.id,
            entity_id: conflict.entity_id,
            entity_type: conflict.entity_type.clone(),
            is_note: matches!(conflict.entity_type, SyncEntityType::Note),
            current: conflict
                .current
                .as_ref()
                .and_then(|record| self.note_from_record(record).ok().flatten()),
            incoming: self.incoming_note(conflict),
            current_revision: conflict.entity_revision,
            incoming_revision: conflict.incoming.base_revision + 1,
        }
    }

    fn incoming_note(&self, conflict: &MutationConflict) -> Option<Note> {
        if !matches!(conflict.entity_type, SyncEntityType::Note) {
            return None;
        }
        let payload = conflict.incoming.encrypted_payload.as_ref()?;
        let bytes = self
            .engine
            .decrypt_payload(&conflict.entity_type, conflict.entity_id, payload)
            .ok()?;
        let parsed = NotePayload::from_bytes(&bytes).ok()?;
        Some(Note::from_payload(
            conflict.entity_id,
            conflict.incoming.base_revision + 1,
            parsed,
        ))
    }

    fn note_from_record(&self, record: &SyncRecord) -> Result<Option<Note>, NoteError> {
        if record.metadata.tombstone
            || !matches!(record.metadata.entity_type, SyncEntityType::Note)
        {
            return Ok(None);
        }
        let bytes = self
            .engine
            .decrypt_record(record)?
            .ok_or(NoteError::Unreadable)?;
        let payload = NotePayload::from_bytes(&bytes)?;
        Ok(Some(Note::from_payload(
            record.metadata.id,
            record.metadata.revision,
            payload,
        )))
    }

    /// Stores the retained incoming version as a new, independent entity.
    fn copy_incoming_version(
        &mut self,
        conflict: &MutationConflict,
    ) -> Result<Option<Uuid>, NoteError> {
        let payload = conflict
            .incoming
            .encrypted_payload
            .as_ref()
            .ok_or(NoteError::ConflictUnreadable)?;
        let bytes = self
            .engine
            .decrypt_payload(&conflict.entity_type, conflict.entity_id, payload)?;
        match conflict.entity_type {
            SyncEntityType::Note => {
                let parsed = NotePayload::from_bytes(&bytes)?;
                Ok(Some(self.write_note(Uuid::new_v4(), &parsed)?.id))
            }
            SyncEntityType::NoteFolder => {
                let parsed = FolderPayload::from_bytes(&bytes)?;
                Ok(Some(self.write_folder(Uuid::new_v4(), &parsed)?.id))
            }
            _ => Ok(None),
        }
    }

    fn require_payload(&mut self, id: Uuid) -> Result<NotePayload, NoteError> {
        match self.read_note(id)? {
            EntityRead::Present(note) => Ok(note.into_payload()),
            EntityRead::Missing => Err(NoteError::NotFound),
            EntityRead::Tombstoned => Err(NoteError::Deleted),
            EntityRead::Unreadable => Err(NoteError::Unreadable),
        }
    }

    fn require_folder_payload(&mut self, id: Uuid) -> Result<FolderPayload, NoteError> {
        match self.read_folder(id)? {
            EntityRead::Present(folder) => Ok(FolderPayload {
                schema_version: FOLDER_PAYLOAD_SCHEMA_VERSION,
                name: folder.name,
                created_at: folder.created_at,
                updated_at: folder.updated_at,
                deleted_at: folder.deleted_at,
            }),
            EntityRead::Missing => Err(NoteError::NotFound),
            EntityRead::Tombstoned => Err(NoteError::Deleted),
            EntityRead::Unreadable => Err(NoteError::Unreadable),
        }
    }

    fn write_note(&mut self, id: Uuid, payload: &NotePayload) -> Result<Note, NoteError> {
        payload.validate()?;
        let bytes = payload.to_bytes()?;
        let mutation = self
            .engine
            .create_local_change(SyncEntityType::Note, id, &bytes)?;
        let revision = self.revision_after(mutation.base_revision, id)?;
        let note = Note::from_payload(id, revision, payload.clone());
        self.cache.upsert_note(note.clone());
        Ok(note)
    }

    fn write_folder(&mut self, id: Uuid, payload: &FolderPayload) -> Result<NoteFolder, NoteError> {
        payload.validate()?;
        let bytes = payload.to_bytes()?;
        let mutation = self
            .engine
            .create_local_change(SyncEntityType::NoteFolder, id, &bytes)?;
        let revision = self.revision_after(mutation.base_revision, id)?;
        let folder = NoteFolder {
            id,
            revision,
            name: payload.name.clone(),
            created_at: payload.created_at.clone(),
            updated_at: payload.updated_at.clone(),
            deleted_at: payload.deleted_at.clone(),
        };
        self.cache.upsert_folder(folder.clone());
        Ok(folder)
    }

    fn revision_after(&self, base_revision: u64, id: Uuid) -> Result<u64, NoteError> {
        // Read back so the reported revision is what storage actually holds.
        match self.engine.record(id)? {
            Some(record) => Ok(record.metadata.revision),
            None => Ok(base_revision + 1),
        }
    }

    fn read_note(&self, id: Uuid) -> Result<EntityRead<Note>, NoteError> {
        let record = match self.engine.record(id)? {
            Some(record) => record,
            None => return Ok(EntityRead::Missing),
        };
        if record.metadata.tombstone {
            return Ok(EntityRead::Tombstoned);
        }
        if !matches!(record.metadata.entity_type, SyncEntityType::Note) {
            return Ok(EntityRead::Missing);
        }
        match self.note_from_record(&record) {
            Ok(Some(note)) => Ok(EntityRead::Present(note)),
            _ => Ok(EntityRead::Unreadable),
        }
    }

    fn read_folder(&self, id: Uuid) -> Result<EntityRead<NoteFolder>, NoteError> {
        let record = match self.engine.record(id)? {
            Some(record) => record,
            None => return Ok(EntityRead::Missing),
        };
        if record.metadata.tombstone {
            return Ok(EntityRead::Tombstoned);
        }
        if !matches!(record.metadata.entity_type, SyncEntityType::NoteFolder) {
            return Ok(EntityRead::Missing);
        }
        let bytes = match self.engine.decrypt_record(&record)? {
            Some(bytes) => bytes,
            None => return Ok(EntityRead::Unreadable),
        };
        match FolderPayload::from_bytes(&bytes) {
            Ok(payload) => Ok(EntityRead::Present(NoteFolder {
                id,
                revision: record.metadata.revision,
                name: payload.name,
                created_at: payload.created_at,
                updated_at: payload.updated_at,
                deleted_at: payload.deleted_at,
            })),
            Err(_) => Ok(EntityRead::Unreadable),
        }
    }

    fn ensure_loaded(&mut self) -> Result<(), NoteError> {
        if self.cache.loaded {
            return Ok(());
        }
        let note_ids = self.entity_ids(&SyncEntityType::Note)?;
        let folder_ids = self.entity_ids(&SyncEntityType::NoteFolder)?;
        let mut cache = NoteCache {
            loaded: true,
            ..NoteCache::default()
        };
        for id in note_ids {
            match self.read_note(id)? {
                EntityRead::Present(note) => cache.notes.push(note),
                EntityRead::Unreadable => cache.unreadable += 1,
                EntityRead::Missing | EntityRead::Tombstoned => {}
            }
        }
        for id in folder_ids {
            if let EntityRead::Present(folder) = self.read_folder(id)? {
                cache.folders.push(folder);
            }
        }
        self.cache = cache;
        Ok(())
    }

    /// Every entity identifier that ever appeared in the applied journal.
    ///
    /// The journal is the only enumeration source, so tombstoned entities are
    /// included here and filtered out by the caller.
    fn entity_ids(&self, entity_type: &SyncEntityType) -> Result<Vec<Uuid>, NoteError> {
        let mut cursor = SyncCursor(0);
        let mut collected: BTreeSet<Uuid> = BTreeSet::new();
        loop {
            let page = self.engine.page_after(cursor, crate::sync::MAX_PAGE_SIZE)?;
            if page.operations.is_empty() {
                break;
            }
            for operation in &page.operations {
                if &operation.mutation.entity_type == entity_type {
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
