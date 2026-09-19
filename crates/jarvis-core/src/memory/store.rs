//! `MemoryStore`: encrypted AI memory on top of the synchronization layer.
//!
//! It reuses the same [`SyncRepository`], `SyncEngine`, `SqliteSyncRepository`,
//! production crypto, and conflict model as notes and the password vault, but:
//!
//! * it lives in its own database file (`ai-memory.sqlite3`), so journals and
//!   cursors never mix;
//! * it uses the four `AiMemory*` entity types and never touches another one, so
//!   a wrong identifier cannot reach a note or a password record;
//! * its payload cipher is a [`PurposeKeyProvider`] for `JARVIS/ai-memory/v1`,
//!   which holds a key derived from the master key and never the master key
//!   itself.
//!
//! Reads decrypt the whole memory set once into memory, which is what makes search
//! work without any plaintext in SQLite. Writes update that cache in place. The
//! first read is linear in the number of stored entries; [`MemoryStore::stats`]
//! reports the size, so the interface can warn about the cost before it grows.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ai::{ChatMessage, Persona};
use crate::sync::crypto::PurposeKeyProvider;
use crate::sync::sqlite::SqliteSyncRepository;
use crate::sync::{
    CryptoProvider, DeviceId, EncryptedPayload, MutationConflict, SyncCursor, SyncEngine,
    SyncEntityType, SyncOperationKind, SyncRecord, SyncRepository, MAX_PAGE_SIZE,
};

use super::error::MemoryError;
use super::model::*;

/// Concrete store used by the application: the memory database with the memory key.
pub type EncryptedMemoryStore = MemoryStore<SqliteSyncRepository, PurposeKeyProvider>;

/// Result of reading one entity from storage.
enum EntityRead<T> {
    Missing,
    Tombstoned,
    Present(T, u64),
    Unreadable,
}

/// The four kinds of stored memory entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MemoryKind {
    Conversation,
    Message,
    Summary,
    Fact,
}

impl MemoryKind {
    fn entity_type(&self) -> SyncEntityType {
        match self {
            Self::Conversation => SyncEntityType::AiMemoryConversation,
            Self::Message => SyncEntityType::AiMemoryMessage,
            Self::Summary => SyncEntityType::AiMemorySummary,
            Self::Fact => SyncEntityType::AiMemoryFact,
        }
    }
}

/// One decrypted entry with its identifier and revision.
#[derive(Clone, Debug)]
struct Entry<T> {
    id: Uuid,
    revision: u64,
    payload: T,
}

/// Decrypted view of the memory database.
#[derive(Default)]
struct MemoryCache {
    loaded: bool,
    conversations: Vec<Entry<ConversationPayload>>,
    messages: Vec<Entry<MessagePayload>>,
    summaries: Vec<Entry<SummaryPayload>>,
    facts: Vec<Entry<FactPayload>>,
    /// Entries that exist but could not be decrypted; reported, never hidden.
    unreadable: usize,
}

impl MemoryCache {
    fn upsert<T>(entries: &mut Vec<Entry<T>>, id: Uuid, revision: u64, payload: T) {
        match entries.iter().position(|entry| entry.id == id) {
            Some(index) => {
                entries[index] = Entry {
                    id,
                    revision,
                    payload,
                };
            }
            None => entries.push(Entry {
                id,
                revision,
                payload,
            }),
        }
    }

    fn remove<T>(entries: &mut Vec<Entry<T>>, id: Uuid) {
        entries.retain(|entry| entry.id != id);
    }
}

/// Encrypted AI memory.
pub struct MemoryStore<R: SyncRepository, C: CryptoProvider> {
    engine: SyncEngine<R, C>,
    cache: MemoryCache,
}

impl<R: SyncRepository, C: CryptoProvider> MemoryStore<R, C> {
    pub fn new(repository: R, crypto: C, device_id: DeviceId) -> Self {
        Self {
            engine: SyncEngine::new(repository, crypto, device_id),
            cache: MemoryCache::default(),
        }
    }

    pub fn device_id(&self) -> &DeviceId {
        self.engine.device_id()
    }

    pub fn repository(&self) -> &R {
        self.engine.repository()
    }

    pub fn repository_mut(&mut self) -> &mut R {
        self.engine.repository_mut()
    }

    pub fn into_repository(self) -> R {
        self.engine.into_repository()
    }

    /// Drops the decrypted cache; the next read reloads and re-decrypts.
    pub fn invalidate(&mut self) {
        self.cache = MemoryCache::default();
    }

    /// Whether decrypted memory is currently held in memory.
    pub fn holds_decrypted_entries(&self) -> bool {
        self.cache.loaded
            && (!self.cache.conversations.is_empty()
                || !self.cache.messages.is_empty()
                || !self.cache.summaries.is_empty()
                || !self.cache.facts.is_empty())
    }

    /// Whether the database holds anything at all. Needs no decryption.
    pub fn has_stored_entities(&self) -> Result<bool, MemoryError> {
        let page = self.engine.page_after(SyncCursor(0), 1)?;
        Ok(!page.operations.is_empty())
    }

    // ------------------------------------------------------------ conversations

    pub fn create_conversation(
        &mut self,
        title: &str,
        profile: Persona,
    ) -> Result<ConversationView, MemoryError> {
        let payload = ConversationPayload::create(title, profile)?;
        let id = Uuid::new_v4();
        let revision = self.write(MemoryKind::Conversation, id, &payload.to_bytes()?)?;
        MemoryCache::upsert(&mut self.cache.conversations, id, revision, payload);
        self.conversation_view(id).ok_or(MemoryError::NotFound)
    }

    pub fn list_conversations(
        &mut self,
        query: &ConversationQuery,
    ) -> Result<Vec<ConversationView>, MemoryError> {
        let query = query.normalized();
        self.ensure_loaded()?;
        let mut views: Vec<ConversationView> = self
            .cache
            .conversations
            .iter()
            .filter(|entry| query.include_archived || !entry.payload.is_archived())
            .map(|entry| self.view_of_conversation(entry))
            .collect();
        // Most recently updated first, then by identifier for a stable order.
        views.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(views
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect())
    }

    pub fn conversation(&mut self, id: Uuid) -> Result<Option<ConversationView>, MemoryError> {
        self.ensure_loaded()?;
        Ok(self.conversation_view(id))
    }

    /// Conversation header, a page of messages, the summary, and pending candidates.
    pub fn open_conversation(
        &mut self,
        id: Uuid,
        offset: usize,
        limit: usize,
    ) -> Result<ConversationDetails, MemoryError> {
        let conversation = self.conversation(id)?.ok_or(MemoryError::NotFound)?;
        let page = self.list_messages(id, offset, limit)?;
        let summary = self.summary(id)?;
        let candidates = self.list_candidates(Some(id))?;
        Ok(ConversationDetails {
            conversation,
            page,
            summary,
            candidates,
        })
    }

    pub fn rename_conversation(
        &mut self,
        id: Uuid,
        title: &str,
    ) -> Result<ConversationView, MemoryError> {
        let mut payload = self.require_conversation(id)?;
        payload.rename(title)?;
        self.write_conversation(id, payload)
    }

    pub fn set_conversation_archived(
        &mut self,
        id: Uuid,
        archived: bool,
    ) -> Result<ConversationView, MemoryError> {
        let mut payload = self.require_conversation(id)?;
        payload.set_archived(archived);
        self.write_conversation(id, payload)
    }

    /// Touches a conversation so the list order follows the latest activity.
    pub fn touch_conversation(&mut self, id: Uuid) -> Result<(), MemoryError> {
        let mut payload = self.require_conversation(id)?;
        payload.touch();
        self.write_conversation(id, payload)?;
        Ok(())
    }

    /// Deletes a conversation and everything stored inside it.
    ///
    /// Messages and the summary are tombstoned too: a deleted conversation must
    /// not leave readable content behind under an identifier nobody can open.
    /// Returns how many related entries were removed with it.
    pub fn delete_conversation(&mut self, id: Uuid) -> Result<usize, MemoryError> {
        self.require_conversation(id)?;
        self.ensure_loaded()?;
        let messages = self.message_ids_of(id);
        let summaries = self.summary_ids_of(id);
        let removed = messages.len() + summaries.len();
        for message in messages {
            self.delete(MemoryKind::Message, message)?;
            MemoryCache::remove(&mut self.cache.messages, message);
        }
        for summary in summaries {
            self.delete(MemoryKind::Summary, summary)?;
            MemoryCache::remove(&mut self.cache.summaries, summary);
        }
        self.delete(MemoryKind::Conversation, id)?;
        MemoryCache::remove(&mut self.cache.conversations, id);
        Ok(removed)
    }

    /// Removes every message and the summary of one conversation, keeping it.
    pub fn clear_conversation(&mut self, id: Uuid) -> Result<usize, MemoryError> {
        self.require_conversation(id)?;
        self.ensure_loaded()?;
        let messages = self.message_ids_of(id);
        let summaries = self.summary_ids_of(id);
        let removed = messages.len() + summaries.len();
        for message in messages {
            self.delete(MemoryKind::Message, message)?;
            MemoryCache::remove(&mut self.cache.messages, message);
        }
        for summary in summaries {
            self.delete(MemoryKind::Summary, summary)?;
            MemoryCache::remove(&mut self.cache.summaries, summary);
        }
        self.touch_conversation(id)?;
        Ok(removed)
    }

    /// Removes every conversation, message, and summary.
    ///
    /// Facts survive on purpose: they are memory the user curated, not history.
    /// The caller must pass an explicit confirmation from the interface.
    pub fn clear_history(&mut self, confirmed: bool) -> Result<usize, MemoryError> {
        if !confirmed {
            return Err(MemoryError::ConfirmationRequired);
        }
        self.ensure_loaded()?;
        let conversations: Vec<Uuid> = self
            .cache
            .conversations
            .iter()
            .map(|entry| entry.id)
            .collect();
        let messages: Vec<Uuid> = self.cache.messages.iter().map(|entry| entry.id).collect();
        let summaries: Vec<Uuid> = self.cache.summaries.iter().map(|entry| entry.id).collect();
        let removed = conversations.len() + messages.len() + summaries.len();
        for id in messages {
            self.delete(MemoryKind::Message, id)?;
        }
        for id in summaries {
            self.delete(MemoryKind::Summary, id)?;
        }
        for id in conversations {
            self.delete(MemoryKind::Conversation, id)?;
        }
        self.cache.conversations.clear();
        self.cache.messages.clear();
        self.cache.summaries.clear();
        Ok(removed)
    }

    // ---------------------------------------------------------------- messages

    /// Stores the user's message.
    pub fn append_user_message(
        &mut self,
        conversation_id: Uuid,
        content: &str,
    ) -> Result<MessageView, MemoryError> {
        self.require_conversation(conversation_id)?;
        let sequence = self.next_sequence(conversation_id)?;
        let payload = MessagePayload::user(conversation_id, content, sequence)?;
        self.write_message(payload)
    }

    /// Stores the visible final answer, or a cancelled or partial one.
    pub fn append_assistant_message(
        &mut self,
        conversation_id: Uuid,
        content: &str,
        status: MessageStatus,
        partial: bool,
    ) -> Result<MessageView, MemoryError> {
        self.require_conversation(conversation_id)?;
        let sequence = self.next_sequence(conversation_id)?;
        let payload =
            MessagePayload::assistant(conversation_id, content, status, partial, sequence)?;
        self.write_message(payload)
    }

    /// Next free position in the conversation.
    fn next_sequence(&mut self, conversation_id: Uuid) -> Result<u64, MemoryError> {
        self.ensure_loaded()?;
        let highest = self
            .cache
            .messages
            .iter()
            .filter(|entry| entry.payload.conversation_id == conversation_id)
            .map(|entry| entry.payload.sequence)
            .max()
            .unwrap_or(0);
        Ok(highest.saturating_add(1))
    }

    fn write_message(&mut self, payload: MessagePayload) -> Result<MessageView, MemoryError> {
        let conversation_id = payload.conversation_id;
        let id = Uuid::new_v4();
        let revision = self.write(MemoryKind::Message, id, &payload.to_bytes()?)?;
        let view = MessageView {
            id,
            revision,
            role: payload.role,
            content: payload.content.clone(),
            status: payload.status,
            partial: payload.partial,
            sequence: payload.sequence,
            created_at: payload.created_at.clone(),
        };
        MemoryCache::upsert(&mut self.cache.messages, id, revision, payload);
        self.touch_conversation(conversation_id)?;
        Ok(view)
    }

    /// Messages of one conversation, oldest first.
    pub fn list_messages(
        &mut self,
        conversation_id: Uuid,
        offset: usize,
        limit: usize,
    ) -> Result<MessagePage, MemoryError> {
        self.ensure_loaded()?;
        let messages = self.messages_of(conversation_id);
        let total = messages.len();
        let limit = if limit == 0 { MAX_PAGE_SIZE } else { limit };
        let page = messages
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(message_view)
            .collect();
        Ok(MessagePage {
            messages: page,
            total,
            offset,
        })
    }

    /// Deletes one message and marks a summary that covered it as stale.
    pub fn delete_message(&mut self, id: Uuid) -> Result<(), MemoryError> {
        let entry = match self.read_ref(MemoryKind::Message, id)? {
            EntityRead::Present(bytes, revision) => {
                let payload = MessagePayload::from_bytes(&bytes)?;
                (payload, revision)
            }
            EntityRead::Missing => return Err(MemoryError::NotFound),
            EntityRead::Tombstoned => return Err(MemoryError::Deleted),
            EntityRead::Unreadable => return Err(MemoryError::Unreadable),
        };
        let conversation_id = entry.0.conversation_id;
        let covered = self.covered_message_ids(conversation_id);
        self.delete(MemoryKind::Message, id)?;
        MemoryCache::remove(&mut self.cache.messages, id);
        if covered.contains(&id) {
            self.mark_summary_stale(conversation_id)?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- summaries

    pub fn summary(&mut self, conversation_id: Uuid) -> Result<Option<SummaryView>, MemoryError> {
        self.ensure_loaded()?;
        Ok(self
            .cache
            .summaries
            .iter()
            .find(|entry| entry.payload.conversation_id == conversation_id)
            .map(summary_view))
    }

    /// Creates or replaces the summary of one conversation.
    pub fn save_summary(
        &mut self,
        conversation_id: Uuid,
        summary: &str,
        covers_until_message: Uuid,
        covered_messages: u32,
    ) -> Result<SummaryView, MemoryError> {
        self.require_conversation(conversation_id)?;
        self.ensure_loaded()?;
        let existing = self
            .cache
            .summaries
            .iter()
            .find(|entry| entry.payload.conversation_id == conversation_id)
            .map(|entry| (entry.id, entry.payload.clone()));

        let (id, payload) = match existing {
            Some((id, mut payload)) => {
                payload.replace(summary, covers_until_message, covered_messages)?;
                (id, payload)
            }
            None => (
                Uuid::new_v4(),
                SummaryPayload::create(
                    conversation_id,
                    summary,
                    covers_until_message,
                    covered_messages,
                )?,
            ),
        };
        let revision = self.write(MemoryKind::Summary, id, &payload.to_bytes()?)?;
        MemoryCache::upsert(&mut self.cache.summaries, id, revision, payload);
        self.summary(conversation_id)?.ok_or(MemoryError::NotFound)
    }

    /// Marks the summary out of date, for example after a covered message changed.
    pub fn mark_summary_stale(&mut self, conversation_id: Uuid) -> Result<bool, MemoryError> {
        self.ensure_loaded()?;
        let Some(entry) = self
            .cache
            .summaries
            .iter()
            .find(|entry| entry.payload.conversation_id == conversation_id)
            .cloned()
        else {
            return Ok(false);
        };
        if entry.payload.stale {
            return Ok(false);
        }
        let mut payload = entry.payload;
        payload.mark_stale();
        let revision = self.write(MemoryKind::Summary, entry.id, &payload.to_bytes()?)?;
        MemoryCache::upsert(&mut self.cache.summaries, entry.id, revision, payload);
        Ok(true)
    }

    pub fn delete_summary(&mut self, conversation_id: Uuid) -> Result<bool, MemoryError> {
        self.ensure_loaded()?;
        let Some(id) = self
            .cache
            .summaries
            .iter()
            .find(|entry| entry.payload.conversation_id == conversation_id)
            .map(|entry| entry.id)
        else {
            return Ok(false);
        };
        self.delete(MemoryKind::Summary, id)?;
        MemoryCache::remove(&mut self.cache.summaries, id);
        Ok(true)
    }

    /// Identifiers a summary currently covers: everything up to its boundary.
    fn covered_message_ids(&self, conversation_id: Uuid) -> Vec<Uuid> {
        let Some(boundary) = self
            .cache
            .summaries
            .iter()
            .find(|entry| entry.payload.conversation_id == conversation_id)
            .map(|entry| entry.payload.covers_until_message)
        else {
            return Vec::new();
        };
        let mut covered = Vec::new();
        for message in self.messages_of(conversation_id) {
            covered.push(message.id);
            if message.id == boundary {
                break;
            }
        }
        covered
    }

    // ------------------------------------------------------------------- facts

    /// Creates a fact the user typed or approved.
    pub fn create_fact(&mut self, draft: &FactDraft) -> Result<FactView, MemoryError> {
        let mut payload = FactPayload::manual(draft.scope, draft.category, &draft.content)?;
        payload.set_usage_flags(draft.pinned, draft.disabled);
        self.write_fact(Uuid::new_v4(), payload)
    }

    /// Stores a model suggestion as a pending candidate.
    ///
    /// A candidate is never usable as context until the user approves it.
    pub fn create_candidate(
        &mut self,
        scope: MemoryScope,
        category: MemoryCategory,
        content: &str,
        confidence: f32,
        conversation_id: Uuid,
        source_message_id: Uuid,
    ) -> Result<FactView, MemoryError> {
        let payload = FactPayload::candidate(
            scope,
            category,
            content,
            confidence,
            conversation_id,
            source_message_id,
        )?;
        self.write_fact(Uuid::new_v4(), payload)
    }

    pub fn list_facts(&mut self, query: &FactQuery) -> Result<Vec<FactView>, MemoryError> {
        let query = query.normalized();
        self.ensure_loaded()?;
        let needle = query.search.to_lowercase();
        let mut views: Vec<FactView> = self
            .cache
            .facts
            .iter()
            .filter(|entry| {
                if !query.include_deleted && entry.payload.is_trashed() {
                    return false;
                }
                if let Some(scope) = query.scope {
                    if entry.payload.scope != scope {
                        return false;
                    }
                }
                if let Some(category) = query.category {
                    if entry.payload.category != category {
                        return false;
                    }
                }
                if let Some(state) = query.state {
                    if entry.payload.state != state {
                        return false;
                    }
                }
                if !needle.is_empty() && !entry.payload.content.to_lowercase().contains(&needle) {
                    return false;
                }
                true
            })
            .map(|entry| FactView::from_payload(entry.id, entry.revision, &entry.payload))
            .collect();
        // Pinned first, then most recently updated.
        views.sort_by(|left, right| {
            right
                .pinned
                .cmp(&left.pinned)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(views
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect())
    }

    /// Every usable fact for a profile.
    ///
    /// This is the only path the context builder uses, and it filters by scope, so
    /// JARVIS cannot see ALTRON's private facts even through a stored identifier.
    pub fn usable_facts(&mut self, persona: Persona) -> Result<Vec<FactView>, MemoryError> {
        self.ensure_loaded()?;
        let mut views: Vec<FactView> = self
            .cache
            .facts
            .iter()
            .filter(|entry| entry.payload.is_usable_for(persona))
            .map(|entry| FactView::from_payload(entry.id, entry.revision, &entry.payload))
            .collect();
        views.sort_by_key(|view| view.id);
        Ok(views)
    }

    pub fn fact(&mut self, id: Uuid) -> Result<Option<FactView>, MemoryError> {
        self.ensure_loaded()?;
        Ok(self
            .cache
            .facts
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| FactView::from_payload(entry.id, entry.revision, &entry.payload)))
    }

    pub fn update_fact(&mut self, id: Uuid, draft: &FactDraft) -> Result<FactView, MemoryError> {
        let mut payload = self.require_fact(id)?;
        payload.apply_edit(
            draft.scope,
            draft.category,
            &draft.content,
            draft.pinned,
            draft.disabled,
        )?;
        self.write_fact(id, payload)
    }

    /// Approves a candidate, optionally after the user corrected it.
    pub fn approve_candidate(
        &mut self,
        id: Uuid,
        draft: Option<&FactDraft>,
    ) -> Result<FactView, MemoryError> {
        let mut payload = self.require_fact(id)?;
        if let Some(draft) = draft {
            payload.apply_edit(
                draft.scope,
                draft.category,
                &draft.content,
                draft.pinned,
                draft.disabled,
            )?;
        }
        payload.approve();
        self.write_fact(id, payload)
    }

    pub fn reject_candidate(&mut self, id: Uuid) -> Result<FactView, MemoryError> {
        let mut payload = self.require_fact(id)?;
        payload.reject();
        self.write_fact(id, payload)
    }

    /// Pins or disables a fact without deleting it.
    pub fn set_fact_usage(
        &mut self,
        id: Uuid,
        pinned: bool,
        disabled: bool,
    ) -> Result<FactView, MemoryError> {
        let mut payload = self.require_fact(id)?;
        payload.set_usage_flags(pinned, disabled);
        self.write_fact(id, payload)
    }

    /// Records that these facts were used to build a context.
    pub fn mark_facts_used(&mut self, ids: &[Uuid]) -> Result<usize, MemoryError> {
        if ids.is_empty() {
            return Ok(0);
        }
        self.ensure_loaded()?;
        let wanted: BTreeSet<Uuid> = ids.iter().copied().collect();
        let mut updated = 0usize;
        for id in wanted {
            let Some(entry) = self
                .cache
                .facts
                .iter()
                .find(|entry| entry.id == id)
                .cloned()
            else {
                continue;
            };
            let mut payload = entry.payload;
            payload.mark_used();
            let revision = self.write(MemoryKind::Fact, entry.id, &payload.to_bytes()?)?;
            MemoryCache::upsert(&mut self.cache.facts, entry.id, revision, payload);
            updated += 1;
        }
        Ok(updated)
    }

    /// Soft-deletes a fact; its text stays recoverable until the user purges it.
    pub fn delete_fact(&mut self, id: Uuid) -> Result<FactView, MemoryError> {
        let mut payload = self.require_fact(id)?;
        if payload.trash() {
            return self.write_fact(id, payload);
        }
        let revision = self.revision_of(id)?;
        Ok(FactView::from_payload(id, revision, &payload))
    }

    pub fn restore_fact(&mut self, id: Uuid) -> Result<FactView, MemoryError> {
        let mut payload = self.require_fact(id)?;
        if payload.restore() {
            return self.write_fact(id, payload);
        }
        let revision = self.revision_of(id)?;
        Ok(FactView::from_payload(id, revision, &payload))
    }

    /// Writes a tombstone: the payload is dropped for good.
    pub fn purge_fact(&mut self, id: Uuid) -> Result<(), MemoryError> {
        self.require_fact(id)?;
        self.delete(MemoryKind::Fact, id)?;
        MemoryCache::remove(&mut self.cache.facts, id);
        Ok(())
    }

    pub fn list_candidates(
        &mut self,
        conversation_id: Option<Uuid>,
    ) -> Result<Vec<FactView>, MemoryError> {
        self.ensure_loaded()?;
        let mut views: Vec<FactView> = self
            .cache
            .facts
            .iter()
            .filter(|entry| entry.payload.state == CandidateState::Pending)
            .filter(|entry| match conversation_id {
                Some(wanted) => entry.payload.source_conversation_id == Some(wanted),
                None => true,
            })
            .map(|entry| FactView::from_payload(entry.id, entry.revision, &entry.payload))
            .collect();
        views.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(views)
    }

    // ------------------------------------------------------------------- stats

    pub fn stats(&mut self) -> Result<MemoryStats, MemoryError> {
        self.ensure_loaded()?;
        Ok(MemoryStats {
            conversations: self.cache.conversations.len(),
            archived: self
                .cache
                .conversations
                .iter()
                .filter(|entry| entry.payload.is_archived())
                .count(),
            messages: self.cache.messages.len(),
            // Only approved, non-deleted entries are "memory"; a pending candidate is
            // counted separately, because it is not usable yet.
            facts: self
                .cache
                .facts
                .iter()
                .filter(|entry| {
                    entry.payload.state == CandidateState::Approved && !entry.payload.is_trashed()
                })
                .count(),
            pending_candidates: self
                .cache
                .facts
                .iter()
                .filter(|entry| {
                    entry.payload.state == CandidateState::Pending && !entry.payload.is_trashed()
                })
                .count(),
            disabled_facts: self
                .cache
                .facts
                .iter()
                .filter(|entry| entry.payload.disabled && !entry.payload.is_trashed())
                .count(),
            trashed_facts: self
                .cache
                .facts
                .iter()
                .filter(|entry| entry.payload.is_trashed())
                .count(),
            unreadable: self.cache.unreadable,
        })
    }

    // ------------------------------------------------------------------ context

    /// Messages of a conversation that no summary covers yet.
    pub fn uncovered_messages(
        &mut self,
        conversation_id: Uuid,
    ) -> Result<Vec<MessageView>, MemoryError> {
        self.ensure_loaded()?;
        let covered = self.covered_message_ids(conversation_id);
        Ok(self
            .messages_of(conversation_id)
            .into_iter()
            .filter(|entry| !covered.contains(&entry.id))
            .map(message_view)
            .collect())
    }

    /// Every message of a conversation, oldest first, as the chat contracts see it.
    pub fn chat_history(&mut self, conversation_id: Uuid) -> Result<Vec<ChatMessage>, MemoryError> {
        self.ensure_loaded()?;
        Ok(self
            .messages_of(conversation_id)
            .into_iter()
            .filter(|entry| entry.payload.is_usable())
            .map(|entry| ChatMessage {
                role: entry.payload.role.to_chat_role(),
                content: entry.payload.content,
            })
            .collect())
    }

    // ---------------------------------------------------------------- conflicts

    /// Retained conflicts that belong to AI memory.
    pub fn conflicts(&self) -> Result<Vec<MemoryConflictView>, MemoryError> {
        let conflicts = self.engine.conflicts()?;
        Ok(conflicts
            .iter()
            .filter(|conflict| conflict.entity_type.is_ai_memory())
            .map(MemoryConflictView::from_conflict)
            .collect())
    }

    /// Resolves a conflict without destroying either stored version.
    ///
    /// `KeepCurrent` forgets the incoming version; `AcceptIncoming` re-applies it
    /// as a new revision. The journal keeps the incoming entry either way, so a
    /// resolution is never silent. `KeepBoth` is not offered for memory: a
    /// duplicated conversation or message has no meaning in the interface.
    pub fn resolve_conflict(
        &mut self,
        conflict_id: Uuid,
        resolution: MemoryConflictResolution,
    ) -> Result<bool, MemoryError> {
        let conflict = self
            .engine
            .conflicts()?
            .into_iter()
            .find(|conflict| conflict.id == conflict_id)
            .ok_or(MemoryError::NotFound)?;
        let entity_id = conflict.entity_id;
        let entity_type = conflict.entity_type.clone();

        if resolution == MemoryConflictResolution::AcceptIncoming {
            let current = self.engine.record(entity_id)?;
            if current
                .as_ref()
                .is_some_and(|record| record.metadata.tombstone)
            {
                // A tombstone cannot be overwritten.
                return Err(MemoryError::Deleted);
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
                .ok_or(MemoryError::Unreadable)?;
            self.engine.submit_encrypted(
                entity_type,
                entity_id,
                base_revision,
                kind,
                Some(payload),
            )?;
        }

        if !self.engine.repository_mut().discard_conflict(conflict_id)? {
            return Err(MemoryError::NotFound);
        }
        self.invalidate();
        Ok(true)
    }

    // ------------------------------------------------------------ export/import

    /// Every stored AI-memory entity, as ciphertext.
    ///
    /// This is a *state snapshot*, not a journal replay: each entity appears once,
    /// with its current ciphertext and tombstone flag, and with a base revision of
    /// zero so it can be imported into an empty database. Nothing is decrypted here,
    /// so the file is useless without the master key (or its portable backup), and
    /// exporting plaintext is deliberately not offered.
    pub fn export_records(&self) -> Result<Vec<ExportedMemoryRecord>, MemoryError> {
        let mut records = Vec::new();
        for (id, entity_type, record) in self.all_records()? {
            records.push(ExportedMemoryRecord {
                entity_id: id,
                entity_type: entity_type.as_str().to_string(),
                base_revision: 0,
                tombstone: record.metadata.tombstone,
                payload: record
                    .content
                    .as_ref()
                    .map(|payload| payload.as_opaque_bytes().to_vec()),
            });
        }
        records.sort_by_key(|record| record.entity_id);
        Ok(records)
    }

    /// Re-applies exported records.
    ///
    /// A record that collides with a different revision becomes a retained
    /// conflict instead of overwriting anything, so an import can never destroy
    /// newer local memory silently.
    pub fn import_records(
        &mut self,
        records: &[ExportedMemoryRecord],
    ) -> Result<MemoryImportOutcome, MemoryError> {
        let mut outcome = MemoryImportOutcome::default();
        for record in records {
            let entity_type = SyncEntityType::from_storage_name(&record.entity_type)?;
            if !entity_type.is_ai_memory() {
                // An export may only carry AI memory; anything else is refused.
                return Err(MemoryError::MalformedPayload);
            }
            let payload = record
                .payload
                .as_ref()
                .map(|bytes| EncryptedPayload::from_opaque_bytes(bytes.clone()));
            let kind = if record.tombstone {
                SyncOperationKind::Delete
            } else if record.base_revision == 0 {
                SyncOperationKind::Create
            } else {
                SyncOperationKind::Update
            };
            match self.engine.submit_encrypted(
                entity_type,
                record.entity_id,
                record.base_revision,
                kind,
                payload,
            ) {
                Ok(_) => outcome.applied += 1,
                Err(crate::sync::SyncError::UnexpectedConflict) => outcome.conflicts += 1,
                Err(error) => return Err(MemoryError::Storage(error)),
            }
        }
        self.invalidate();
        Ok(outcome)
    }

    // --------------------------------------------------------------- internals

    fn conversation_view(&self, id: Uuid) -> Option<ConversationView> {
        self.cache
            .conversations
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| self.view_of_conversation(entry))
    }

    fn view_of_conversation(&self, entry: &Entry<ConversationPayload>) -> ConversationView {
        let mut message_count = 0usize;
        let mut first: Option<&str> = None;
        let mut last: Option<&str> = None;
        for message in &self.cache.messages {
            if message.payload.conversation_id != entry.id {
                continue;
            }
            message_count += 1;
            let stamp = message.payload.created_at.as_str();
            if first.is_none_or(|current| stamp < current) {
                first = Some(stamp);
            }
            if last.is_none_or(|current| stamp > current) {
                last = Some(stamp);
            }
        }
        ConversationView {
            id: entry.id,
            revision: entry.revision,
            title: entry.payload.title.clone(),
            profile: entry.payload.profile,
            created_at: entry.payload.created_at.clone(),
            updated_at: entry.payload.updated_at.clone(),
            archived_at: entry.payload.archived_at.clone(),
            message_count,
            first_message_at: first.map(str::to_string),
            last_message_at: last.map(str::to_string),
        }
    }

    fn messages_of(&self, conversation_id: Uuid) -> Vec<Entry<MessagePayload>> {
        let mut messages: Vec<Entry<MessagePayload>> = self
            .cache
            .messages
            .iter()
            .filter(|entry| entry.payload.conversation_id == conversation_id)
            .cloned()
            .collect();
        // The stored sequence is the conversation order: it does not depend on the
        // resolution of the system clock, so two messages written in the same tick
        // keep the order they were written in.
        messages.sort_by(|left, right| {
            left.payload
                .sequence
                .cmp(&right.payload.sequence)
                .then_with(|| left.id.cmp(&right.id))
        });
        messages
    }

    fn message_ids_of(&self, conversation_id: Uuid) -> Vec<Uuid> {
        self.cache
            .messages
            .iter()
            .filter(|entry| entry.payload.conversation_id == conversation_id)
            .map(|entry| entry.id)
            .collect()
    }

    fn summary_ids_of(&self, conversation_id: Uuid) -> Vec<Uuid> {
        self.cache
            .summaries
            .iter()
            .filter(|entry| entry.payload.conversation_id == conversation_id)
            .map(|entry| entry.id)
            .collect()
    }

    fn require_conversation(&mut self, id: Uuid) -> Result<ConversationPayload, MemoryError> {
        match self.read_ref(MemoryKind::Conversation, id)? {
            EntityRead::Present(bytes, _) => ConversationPayload::from_bytes(&bytes),
            EntityRead::Missing => Err(MemoryError::NotFound),
            EntityRead::Tombstoned => Err(MemoryError::Deleted),
            EntityRead::Unreadable => Err(MemoryError::Unreadable),
        }
    }

    fn require_fact(&mut self, id: Uuid) -> Result<FactPayload, MemoryError> {
        match self.read_ref(MemoryKind::Fact, id)? {
            EntityRead::Present(bytes, _) => FactPayload::from_bytes(&bytes),
            EntityRead::Missing => Err(MemoryError::NotFound),
            EntityRead::Tombstoned => Err(MemoryError::Deleted),
            EntityRead::Unreadable => Err(MemoryError::Unreadable),
        }
    }

    fn revision_of(&self, id: Uuid) -> Result<u64, MemoryError> {
        Ok(self
            .engine
            .record(id)?
            .map(|record| record.metadata.revision)
            .unwrap_or(0))
    }

    fn write_conversation(
        &mut self,
        id: Uuid,
        payload: ConversationPayload,
    ) -> Result<ConversationView, MemoryError> {
        let revision = self.write(MemoryKind::Conversation, id, &payload.to_bytes()?)?;
        MemoryCache::upsert(&mut self.cache.conversations, id, revision, payload);
        self.conversation_view(id).ok_or(MemoryError::NotFound)
    }

    fn write_fact(&mut self, id: Uuid, payload: FactPayload) -> Result<FactView, MemoryError> {
        let revision = self.write(MemoryKind::Fact, id, &payload.to_bytes()?)?;
        let view = FactView::from_payload(id, revision, &payload);
        MemoryCache::upsert(&mut self.cache.facts, id, revision, payload);
        Ok(view)
    }

    fn write(&mut self, kind: MemoryKind, id: Uuid, plaintext: &[u8]) -> Result<u64, MemoryError> {
        self.engine
            .create_local_change(kind.entity_type(), id, plaintext)?;
        self.revision_of(id)
    }

    fn delete(&mut self, kind: MemoryKind, id: Uuid) -> Result<(), MemoryError> {
        self.engine.create_local_delete(kind.entity_type(), id)?;
        Ok(())
    }

    /// Reads one record, refusing anything that is not this kind of AI memory.
    fn read_ref(&self, kind: MemoryKind, id: Uuid) -> Result<EntityRead<Vec<u8>>, MemoryError> {
        let record = match self.engine.record(id)? {
            Some(record) => record,
            None => return Ok(EntityRead::Missing),
        };
        if record.metadata.entity_type != kind.entity_type() {
            // Another feature's record, or another kind of memory entry: refuse
            // rather than report a misleading "missing".
            return Ok(EntityRead::Unreadable);
        }
        if record.metadata.tombstone {
            return Ok(EntityRead::Tombstoned);
        }
        let bytes = match self.engine.decrypt_record(&record) {
            Ok(Some(bytes)) => bytes,
            Ok(None) | Err(_) => return Ok(EntityRead::Unreadable),
        };
        Ok(EntityRead::Present(bytes, record.metadata.revision))
    }

    fn ensure_loaded(&mut self) -> Result<(), MemoryError> {
        if self.cache.loaded {
            return Ok(());
        }
        let mut cache = MemoryCache {
            loaded: true,
            ..MemoryCache::default()
        };
        for (id, entity_type, record) in self.all_records()? {
            if record.metadata.tombstone {
                continue;
            }
            let bytes = match self.engine.decrypt_record(&record) {
                Ok(Some(bytes)) => bytes,
                Ok(None) | Err(_) => {
                    cache.unreadable += 1;
                    continue;
                }
            };
            let revision = record.metadata.revision;
            match entity_type {
                SyncEntityType::AiMemoryConversation => {
                    match ConversationPayload::from_bytes(&bytes) {
                        Ok(payload) => cache.conversations.push(Entry {
                            id,
                            revision,
                            payload,
                        }),
                        Err(_) => cache.unreadable += 1,
                    }
                }
                SyncEntityType::AiMemoryMessage => match MessagePayload::from_bytes(&bytes) {
                    Ok(payload) => cache.messages.push(Entry {
                        id,
                        revision,
                        payload,
                    }),
                    Err(_) => cache.unreadable += 1,
                },
                SyncEntityType::AiMemorySummary => match SummaryPayload::from_bytes(&bytes) {
                    Ok(payload) => cache.summaries.push(Entry {
                        id,
                        revision,
                        payload,
                    }),
                    Err(_) => cache.unreadable += 1,
                },
                SyncEntityType::AiMemoryFact => match FactPayload::from_bytes(&bytes) {
                    Ok(payload) => cache.facts.push(Entry {
                        id,
                        revision,
                        payload,
                    }),
                    Err(_) => cache.unreadable += 1,
                },
                // Not a memory entity: the store never reads it.
                _ => {}
            }
        }
        self.cache = cache;
        Ok(())
    }

    /// Every AI-memory record with its identifier and declared type.
    fn all_records(&self) -> Result<Vec<(Uuid, SyncEntityType, SyncRecord)>, MemoryError> {
        let mut cursor = SyncCursor(0);
        let mut ids: BTreeSet<Uuid> = BTreeSet::new();
        loop {
            let page = self.engine.page_after(cursor, MAX_PAGE_SIZE)?;
            if page.operations.is_empty() {
                break;
            }
            for operation in &page.operations {
                if operation.mutation.entity_type.is_ai_memory() {
                    ids.insert(operation.mutation.entity_id);
                }
            }
            if page.next_cursor <= cursor {
                break;
            }
            cursor = page.next_cursor;
        }
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(record) = self.engine.record(id)? {
                let entity_type = record.metadata.entity_type.clone();
                if !entity_type.is_ai_memory() {
                    continue;
                }
                records.push((id, entity_type, record));
            }
        }
        Ok(records)
    }
}

/// One message entry as the view type.
fn message_view(entry: Entry<MessagePayload>) -> MessageView {
    MessageView {
        id: entry.id,
        revision: entry.revision,
        role: entry.payload.role,
        content: entry.payload.content,
        status: entry.payload.status,
        partial: entry.payload.partial,
        sequence: entry.payload.sequence,
        created_at: entry.payload.created_at,
    }
}

/// One summary entry as the view type.
fn summary_view(entry: &Entry<SummaryPayload>) -> SummaryView {
    SummaryView {
        id: entry.id,
        revision: entry.revision,
        summary: entry.payload.summary.clone(),
        covers_until_message: entry.payload.covers_until_message,
        covered_messages: entry.payload.covered_messages,
        stale: entry.payload.stale,
        created_at: entry.payload.created_at.clone(),
        updated_at: entry.payload.updated_at.clone(),
    }
}

/// How a retained memory conflict can be resolved.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryConflictResolution {
    /// Forget the incoming version and keep what is stored.
    KeepCurrent,
    /// Re-apply the incoming version as a new revision.
    AcceptIncoming,
}

/// One retained conflict, without any decrypted content.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MemoryConflictView {
    pub conflict_id: Uuid,
    pub entity_id: Uuid,
    pub entity_type: String,
    pub current_revision: u64,
    pub incoming_revision: u64,
    /// Whether the incoming side carries a payload that can be re-applied.
    pub incoming_available: bool,
}

impl MemoryConflictView {
    fn from_conflict(conflict: &MutationConflict) -> Self {
        Self {
            conflict_id: conflict.id,
            entity_id: conflict.entity_id,
            entity_type: conflict.entity_type.as_str().to_string(),
            current_revision: conflict.entity_revision,
            incoming_revision: conflict.incoming.base_revision + 1,
            incoming_available: conflict.incoming.encrypted_payload.is_some(),
        }
    }
}

/// Result of importing an encrypted export.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct MemoryImportOutcome {
    pub applied: usize,
    pub conflicts: usize,
    pub skipped: usize,
}
