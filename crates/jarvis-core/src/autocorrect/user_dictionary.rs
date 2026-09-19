//! The user's own word list: encrypted, searchable, and independent of any dictionary.
//!
//! The list lives in its own database file (`autocorrect.sqlite3`) behind
//! [`KeyPurpose::Autocorrect`](crate::sync::crypto::KeyPurpose::Autocorrect), and it
//! reuses exactly the storage stack of notes, the vault, and AI memory: the same
//! [`SyncRepository`], `SyncEngine`, production crypto, revisions, tombstones, and
//! conflict model. One entity type
//! ([`SyncEntityType::AutocorrectDictionary`]) is written and read here, so a wrong
//! identifier cannot reach a note, a password record, or a memory entry.
//!
//! What the plaintext of a word list means for privacy: a user dictionary is a list of
//! the words a person uses, which is why it is never stored in the clear and never sent
//! anywhere. The payload is JSON inside the authenticated encryption, `Debug` redacts
//! it, and every error from this module is content-free.
//!
//! The store also holds a **session ignore list**: words the user silenced for this
//! unlocked session only. It is deliberately *not* persisted, so "ignore once" stays a
//! decision about the current text and not a permanent rule.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::sync::crypto::PurposeKeyProvider;
use crate::sync::sqlite::SqliteSyncRepository;
use crate::sync::{
    CryptoProvider, DeviceId, EncryptedPayload, MutationConflict, SyncCursor, SyncEngine,
    SyncEntityType, SyncOperationKind, SyncRepository, MAX_PAGE_SIZE,
};

use super::error::AutocorrectError;
use super::model::{
    normalize_word, yo_variant, Language, UserDictionaryEntry, UserDictionaryPayload,
    MAX_IGNORED_WORDS, MAX_USER_DICTIONARY_ENTRIES,
};

/// Concrete store used by the application: dictionary database with the dictionary key.
pub type EncryptedUserDictionary = UserDictionaryStore<SqliteSyncRepository, PurposeKeyProvider>;

/// One decrypted dictionary entry with its identifier and revision.
#[derive(Clone, Debug)]
struct Entry {
    id: Uuid,
    revision: u64,
    payload: UserDictionaryPayload,
}

/// How to filter the word list.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UserDictionaryQuery {
    /// Case-insensitive substring of a word.
    pub search: String,
    pub language: Option<Language>,
    /// Only words that came from an import.
    pub imported_only: bool,
    pub offset: usize,
    pub limit: usize,
}

impl UserDictionaryQuery {
    fn normalized(mut self) -> Self {
        self.search = self.search.trim().to_lowercase();
        if self.limit == 0 {
            self.limit = 100;
        }
        self.limit = self.limit.min(500);
        self
    }
}

/// Size and shape of the stored word list.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct UserDictionaryStats {
    pub words: usize,
    pub russian: usize,
    pub english: usize,
    pub imported: usize,
    /// Words silenced for this session only.
    pub ignored: usize,
    /// Entries that exist but could not be decrypted.
    pub unreadable: usize,
    /// Whether decrypted words are currently held in memory.
    pub decrypted_in_memory: bool,
}

/// Result of importing words from a text file.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ImportOutcome {
    pub added: usize,
    /// Words already in the list; importing is idempotent.
    pub duplicates: usize,
    /// Lines that are not a word (empty, too long, not letters).
    pub invalid: usize,
    /// Importing stopped because the list is full.
    pub limit_reached: bool,
}

/// Result of importing encrypted records.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DictionaryImportOutcome {
    pub applied: usize,
    pub conflicts: usize,
}

/// One exported entry, still encrypted.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExportedDictionaryRecord {
    pub entity_id: Uuid,
    pub base_revision: u64,
    pub tombstone: bool,
    pub payload: Option<Vec<u8>>,
}

/// The encrypted user dictionary.
pub struct UserDictionaryStore<R: SyncRepository, C: CryptoProvider> {
    engine: SyncEngine<R, C>,
    entries: Vec<Entry>,
    ignored: BTreeSet<String>,
    loaded: bool,
    unreadable: usize,
}

impl<R: SyncRepository, C: CryptoProvider> UserDictionaryStore<R, C> {
    pub fn new(repository: R, crypto: C, device_id: DeviceId) -> Self {
        Self {
            engine: SyncEngine::new(repository, crypto, device_id),
            entries: Vec::new(),
            ignored: BTreeSet::new(),
            loaded: false,
            unreadable: 0,
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

    /// Drops the decrypted words and the session ignore list.
    ///
    /// Called when the storage is locked: the derived key is dropped with the master
    /// key, and no decrypted word survives in memory.
    pub fn invalidate(&mut self) {
        self.entries.clear();
        self.ignored.clear();
        self.loaded = false;
        self.unreadable = 0;
    }

    /// Whether decrypted words are currently held in memory.
    pub fn holds_decrypted_entries(&self) -> bool {
        self.loaded && !self.entries.is_empty()
    }

    /// Whether the database holds anything at all. Needs no decryption.
    pub fn has_stored_entities(&self) -> Result<bool, AutocorrectError> {
        let page = self.engine.page_after(SyncCursor(0), 1)?;
        Ok(!page.operations.is_empty())
    }

    // ------------------------------------------------------------------- writes

    /// Adds one word. Adding the same word twice is refused, so a list stays unique.
    pub fn add(
        &mut self,
        word: &str,
        language: Language,
        imported: bool,
    ) -> Result<UserDictionaryEntry, AutocorrectError> {
        let payload = UserDictionaryPayload::create(word, language, imported)?;
        if self.find_word(&payload.word, language).is_some() {
            return Err(AutocorrectError::Duplicate);
        }
        let count = self.stats()?.words;
        if count >= MAX_USER_DICTIONARY_ENTRIES {
            return Err(AutocorrectError::DictionaryFull {
                limit: MAX_USER_DICTIONARY_ENTRIES,
            });
        }
        self.write(Uuid::new_v4(), payload)
    }

    /// Adds many words, skipping the ones that are already stored.
    pub fn add_many(
        &mut self,
        words: &[String],
        language: Language,
        imported: bool,
    ) -> Result<ImportOutcome, AutocorrectError> {
        self.ensure_loaded()?;
        let mut outcome = ImportOutcome::default();
        for word in words {
            if self.stats()?.words >= MAX_USER_DICTIONARY_ENTRIES {
                outcome.limit_reached = true;
                break;
            }
            match normalize_word(word) {
                Ok(normalized) => {
                    if self.find_word(&normalized, language).is_some() {
                        outcome.duplicates += 1;
                        continue;
                    }
                    let payload = UserDictionaryPayload::create(&normalized, language, imported)?;
                    self.write(Uuid::new_v4(), payload)?;
                    outcome.added += 1;
                }
                Err(_) => outcome.invalid += 1,
            }
        }
        Ok(outcome)
    }

    /// Removes one entry by identifier.
    pub fn remove(&mut self, id: Uuid) -> Result<(), AutocorrectError> {
        if !self.entries.iter().any(|entry| entry.id == id) {
            return Err(AutocorrectError::NotFound);
        }
        self.delete(id)?;
        self.entries.retain(|entry| entry.id != id);
        Ok(())
    }

    /// Removes one word in one language, by spelling.
    pub fn remove_word(
        &mut self,
        word: &str,
        language: Language,
    ) -> Result<UserDictionaryEntry, AutocorrectError> {
        let normalized = normalize_word(word)?;
        let entry = self
            .find_word(&normalized, language)
            .cloned()
            .ok_or(AutocorrectError::NotFound)?;
        self.delete(entry.id)?;
        self.entries.retain(|candidate| candidate.id != entry.id);
        Ok(self.view_of(&entry))
    }

    /// Removes every word. The caller must pass an explicit confirmation.
    pub fn clear(&mut self, confirmed: bool) -> Result<usize, AutocorrectError> {
        if !confirmed {
            return Err(AutocorrectError::InvalidConfiguration);
        }
        self.ensure_loaded()?;
        let ids: Vec<Uuid> = self.entries.iter().map(|entry| entry.id).collect();
        let removed = ids.len();
        for id in ids {
            self.delete(id)?;
        }
        self.entries.clear();
        Ok(removed)
    }

    // -------------------------------------------------------------------- reads

    /// Every stored word, filtered and paged.
    pub fn list(
        &mut self,
        query: &UserDictionaryQuery,
    ) -> Result<Vec<UserDictionaryEntry>, AutocorrectError> {
        let query = query.clone().normalized();
        self.ensure_loaded()?;
        let mut views: Vec<UserDictionaryEntry> = self
            .entries
            .iter()
            .filter(|entry| {
                if let Some(language) = query.language {
                    if entry.payload.language != language {
                        return false;
                    }
                }
                if query.imported_only && !entry.payload.imported {
                    return false;
                }
                if !query.search.is_empty() && !entry.payload.word.contains(&query.search) {
                    return false;
                }
                true
            })
            .map(|entry| self.view_of(entry))
            .collect();
        // Alphabetical by word, then by language: a word list is meant to be read.
        views.sort_by(|left, right| {
            left.word
                .cmp(&right.word)
                .then_with(|| left.language.cmp(&right.language))
        });
        Ok(views
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .collect())
    }

    /// Loads the decrypted words.
    ///
    /// A caller that wants many read-only queries — a check asks about every word of a
    /// text — loads the list once and then uses [`UserDictionaryStore::known`],
    /// [`UserDictionaryStore::is_ignored`], and
    /// [`UserDictionaryStore::loaded_suggestions`], none of which need a mutable borrow.
    /// Nothing is decrypted twice, and the caller holds no mutable borrow while it walks a
    /// document.
    pub fn prepare(&mut self) -> Result<(), AutocorrectError> {
        self.ensure_loaded()?;
        Ok(())
    }

    /// Whether the loaded list knows a word (or its `ё`/`е` variant).
    ///
    /// [`UserDictionaryStore::prepare`] must have run first. An unloaded list knows
    /// nothing, which is exactly what "the storage is locked" means.
    pub fn known(&self, word: &str) -> bool {
        match normalize_word(word) {
            Ok(normalized) => self.find_word_index(&normalized).is_some(),
            // A word that could not be stored (digits, punctuation) is simply not in the
            // list, and that is not an error here.
            Err(_) => false,
        }
    }

    /// Whether the store knows a word (or its `ё`/`е` variant) in any language.
    ///
    /// This is the "known word" test the checker uses before it reports a typo.
    pub fn is_known(&mut self, word: &str) -> Result<bool, AutocorrectError> {
        self.ensure_loaded()?;
        Ok(self.known(word))
    }

    /// Whether the user silenced this word for the current session.
    pub fn is_ignored(&self, word: &str) -> bool {
        let normalized = word.trim().to_lowercase();
        self.ignored.contains(&normalized)
    }

    /// Silences a word for this session only.
    pub fn ignore_word(&mut self, word: &str) -> Result<(), AutocorrectError> {
        let normalized = normalize_word(word)?;
        if self.ignored.len() >= MAX_IGNORED_WORDS {
            return Err(AutocorrectError::DictionaryFull {
                limit: MAX_IGNORED_WORDS,
            });
        }
        self.ignored.insert(normalized);
        Ok(())
    }

    /// Forgets a session ignore.
    pub fn unignore_word(&mut self, word: &str) -> Result<bool, AutocorrectError> {
        let normalized = normalize_word(word)?;
        Ok(self.ignored.remove(&normalized))
    }

    /// The session ignore list, sorted.
    pub fn ignored_words(&self) -> Vec<String> {
        self.ignored.iter().cloned().collect()
    }

    /// Words that look like a misspelling of `word`, nearest first.
    ///
    /// The scan is bounded: only words of a similar length are compared, the distance is
    /// capped at two edits, and at most `limit` results are returned, so a long list
    /// cannot turn one keystroke into a slow frame.
    pub fn suggestions(
        &mut self,
        word: &str,
        language: Language,
        limit: usize,
    ) -> Result<Vec<String>, AutocorrectError> {
        self.ensure_loaded()?;
        Ok(self.loaded_suggestions(word, language, limit))
    }

    /// Suggestions from the loaded list; [`UserDictionaryStore::prepare`] must have run.
    pub fn loaded_suggestions(&self, word: &str, language: Language, limit: usize) -> Vec<String> {
        if limit == 0 {
            return Vec::new();
        }
        let target = match normalize_word(word) {
            Ok(word) => word,
            Err(_) => return Vec::new(),
        };
        let target_chars = target.chars().count();
        let mut scored: Vec<(usize, String)> = Vec::new();
        for entry in &self.entries {
            if entry.payload.language != language {
                continue;
            }
            let candidate = &entry.payload.word;
            let candidate_chars = candidate.chars().count();
            if candidate_chars + 2 < target_chars || target_chars + 2 < candidate_chars {
                continue;
            }
            let Some(distance) = bounded_distance(&target, candidate, 2) else {
                continue;
            };
            if distance == 0 {
                continue;
            }
            scored.push((distance, candidate.clone()));
        }
        scored.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
        scored.truncate(limit);
        scored.into_iter().map(|(_, word)| word).collect()
    }

    /// Size and shape of the list.
    pub fn stats(&mut self) -> Result<UserDictionaryStats, AutocorrectError> {
        self.ensure_loaded()?;
        Ok(UserDictionaryStats {
            words: self.entries.len(),
            russian: self
                .entries
                .iter()
                .filter(|entry| entry.payload.language == Language::Russian)
                .count(),
            english: self
                .entries
                .iter()
                .filter(|entry| entry.payload.language == Language::English)
                .count(),
            imported: self
                .entries
                .iter()
                .filter(|entry| entry.payload.imported)
                .count(),
            ignored: self.ignored.len(),
            unreadable: self.unreadable,
            decrypted_in_memory: self.holds_decrypted_entries(),
        })
    }

    /// Whether the database holds words, without decrypting them.
    pub fn is_empty(&self) -> Result<bool, AutocorrectError> {
        Ok(!self.has_stored_entities()?)
    }

    // ------------------------------------------------------------ conflicts/export

    /// Retained conflicts that belong to the user dictionary.
    pub fn conflicts(&self) -> Result<Vec<DictionaryConflictView>, AutocorrectError> {
        Ok(self
            .engine
            .conflicts()?
            .iter()
            .filter(|conflict| conflict.entity_type == SyncEntityType::AutocorrectDictionary)
            .map(DictionaryConflictView::from_conflict)
            .collect())
    }

    /// Resolves a conflict without destroying either stored version.
    pub fn resolve_conflict(
        &mut self,
        conflict_id: Uuid,
        accept_incoming: bool,
    ) -> Result<bool, AutocorrectError> {
        let conflict = self
            .engine
            .conflicts()?
            .into_iter()
            .find(|conflict| conflict.id == conflict_id)
            .ok_or(AutocorrectError::NotFound)?;
        if conflict.entity_type != SyncEntityType::AutocorrectDictionary {
            return Err(AutocorrectError::NotFound);
        }
        if accept_incoming {
            let current = self.engine.record(conflict.entity_id)?;
            if current
                .as_ref()
                .is_some_and(|record| record.metadata.tombstone)
            {
                // A tombstone is never overwritten: the user removed the word.
                return Err(AutocorrectError::NotFound);
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
                .ok_or(AutocorrectError::NotFound)?;
            self.engine.submit_encrypted(
                SyncEntityType::AutocorrectDictionary,
                conflict.entity_id,
                base_revision,
                kind,
                Some(payload),
            )?;
        }
        if !self.engine.repository_mut().discard_conflict(conflict_id)? {
            return Err(AutocorrectError::NotFound);
        }
        self.invalidate();
        Ok(true)
    }

    /// Every stored entry, as ciphertext.
    ///
    /// A state snapshot, not a journal replay: each word appears once, with a base
    /// revision of zero so it can be imported into an empty database. Nothing is
    /// decrypted, so the exported file is useless without the master key.
    pub fn export_records(&self) -> Result<Vec<ExportedDictionaryRecord>, AutocorrectError> {
        let mut records = Vec::new();
        for (id, record) in self.all_records()? {
            records.push(ExportedDictionaryRecord {
                entity_id: id,
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

    /// Re-applies exported records. A collision becomes a retained conflict.
    pub fn import_records(
        &mut self,
        records: &[ExportedDictionaryRecord],
    ) -> Result<DictionaryImportOutcome, AutocorrectError> {
        let mut outcome = DictionaryImportOutcome::default();
        for record in records {
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
                SyncEntityType::AutocorrectDictionary,
                record.entity_id,
                record.base_revision,
                kind,
                payload,
            ) {
                Ok(_) => outcome.applied += 1,
                Err(crate::sync::SyncError::UnexpectedConflict) => outcome.conflicts += 1,
                Err(error) => return Err(AutocorrectError::Storage(error)),
            }
        }
        self.invalidate();
        Ok(outcome)
    }

    /// The stored words as plain lines, for an explicit user export.
    ///
    /// This is the only path that produces plaintext, it is never called
    /// automatically, and the interface warns that the resulting file is unprotected.
    pub fn export_words(&mut self) -> Result<Vec<String>, AutocorrectError> {
        self.ensure_loaded()?;
        let mut words: Vec<String> = self
            .entries
            .iter()
            .map(|entry| entry.payload.word.clone())
            .collect();
        words.sort();
        words.dedup();
        Ok(words)
    }

    /// Parses a plain word list, accepting both one-word-per-line files and
    /// Hunspell-style `.dic` headers.
    pub fn parse_word_list(text: &str) -> Vec<String> {
        parse_word_list(text)
    }

    // ---------------------------------------------------------------- internals

    fn view_of(&self, entry: &Entry) -> UserDictionaryEntry {
        UserDictionaryEntry {
            id: entry.id,
            revision: entry.revision,
            word: entry.payload.word.clone(),
            language: entry.payload.language,
            imported: entry.payload.imported,
            created_at: entry.payload.created_at.clone(),
        }
    }

    /// Position of a stored word, comparing `ё`/`е` variants as equal.
    fn find_word_index(&self, normalized: &str) -> Option<usize> {
        let variant = yo_variant(normalized);
        self.entries.iter().position(|entry| {
            entry.payload.word == normalized
                || variant
                    .as_ref()
                    .is_some_and(|variant| &entry.payload.word == variant)
        })
    }

    fn find_word(&self, normalized: &str, language: Language) -> Option<&Entry> {
        let variant = yo_variant(normalized);
        self.entries.iter().find(|entry| {
            entry.payload.language == language
                && (entry.payload.word == normalized
                    || variant
                        .as_ref()
                        .is_some_and(|variant| &entry.payload.word == variant))
        })
    }

    fn write(
        &mut self,
        id: Uuid,
        payload: UserDictionaryPayload,
    ) -> Result<UserDictionaryEntry, AutocorrectError> {
        let bytes = payload.to_bytes()?;
        self.engine
            .create_local_change(SyncEntityType::AutocorrectDictionary, id, &bytes)?;
        let revision = self.revision_of(id)?;
        let entry = Entry {
            id,
            revision,
            payload,
        };
        match self.entries.iter().position(|candidate| candidate.id == id) {
            Some(index) => self.entries[index] = entry.clone(),
            None => self.entries.push(entry.clone()),
        }
        Ok(self.view_of(&entry))
    }

    fn delete(&mut self, id: Uuid) -> Result<(), AutocorrectError> {
        self.engine
            .create_local_delete(SyncEntityType::AutocorrectDictionary, id)?;
        Ok(())
    }

    fn revision_of(&self, id: Uuid) -> Result<u64, AutocorrectError> {
        Ok(self
            .engine
            .record(id)?
            .map(|record| record.metadata.revision)
            .unwrap_or(0))
    }

    fn ensure_loaded(&mut self) -> Result<(), AutocorrectError> {
        if self.loaded {
            return Ok(());
        }
        let mut entries = Vec::new();
        let mut unreadable = 0usize;
        for (id, record) in self.all_records()? {
            if record.metadata.tombstone {
                continue;
            }
            let revision = record.metadata.revision;
            match self.engine.decrypt_record(&record) {
                Ok(Some(bytes)) => match UserDictionaryPayload::from_bytes(&bytes) {
                    Ok(payload) => entries.push(Entry {
                        id,
                        revision,
                        payload,
                    }),
                    Err(_) => unreadable += 1,
                },
                Ok(None) | Err(_) => unreadable += 1,
            }
        }
        self.entries = entries;
        self.unreadable = unreadable;
        self.loaded = true;
        Ok(())
    }

    /// Every dictionary record with its identifier.
    fn all_records(&self) -> Result<Vec<(Uuid, crate::sync::SyncRecord)>, AutocorrectError> {
        let mut cursor = SyncCursor(0);
        let mut ids: BTreeSet<Uuid> = BTreeSet::new();
        loop {
            let page = self.engine.page_after(cursor, MAX_PAGE_SIZE)?;
            if page.operations.is_empty() {
                break;
            }
            for operation in &page.operations {
                if operation.mutation.entity_type == SyncEntityType::AutocorrectDictionary {
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
                if record.metadata.entity_type != SyncEntityType::AutocorrectDictionary {
                    // Another feature's record: never read it from here.
                    continue;
                }
                records.push((id, record));
            }
        }
        Ok(records)
    }
}

/// Parses a plain word list.
///
/// Both shapes a user is likely to have are accepted: one word per line, and a
/// Hunspell-style `.dic` file whose first line is a count. Blank lines and `#` comments
/// are ignored, a tab-separated affix column is dropped, and a line that carries no
/// letter is not a word.
pub fn parse_word_list(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('\t').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter(|line| line.chars().any(char::is_alphabetic))
        .map(str::to_string)
        .collect()
}

/// Damerau-Levenshtein distance, capped: `None` when it would exceed `max`.
///
/// Two rows of the matrix are kept, so the cost is linear in memory and bounded by
/// `max`, and a transposition counts as one edit — which is the mistake a fast typist
/// actually makes.
pub fn bounded_distance(left: &str, right: &str, max: usize) -> Option<usize> {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    if left.len().abs_diff(right.len()) > max {
        return None;
    }
    if left == right {
        return Some(0);
    }
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current: Vec<usize> = vec![0; right.len() + 1];
    let mut before_previous: Vec<usize> = vec![0; right.len() + 1];
    for (row, left_char) in left.iter().enumerate() {
        current[0] = row + 1;
        let mut row_minimum = current[0];
        for (column, right_char) in right.iter().enumerate() {
            let cost = usize::from(left_char != right_char);
            let mut value = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + cost);
            if row > 0
                && column > 0
                && *left_char == right[column - 1]
                && left[row - 1] == *right_char
            {
                value = value.min(before_previous[column - 1] + 1);
            }
            current[column + 1] = value;
            row_minimum = row_minimum.min(value);
        }
        if row_minimum > max {
            return None;
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }
    let distance = previous[right.len()];
    (distance <= max).then_some(distance)
}

/// One retained dictionary conflict, without any decrypted content.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DictionaryConflictView {
    pub conflict_id: Uuid,
    pub entity_id: Uuid,
    pub current_revision: u64,
    pub incoming_revision: u64,
    pub incoming_available: bool,
}

impl DictionaryConflictView {
    fn from_conflict(conflict: &MutationConflict) -> Self {
        Self {
            conflict_id: conflict.id,
            entity_id: conflict.entity_id,
            current_revision: conflict.entity_revision,
            incoming_revision: conflict.incoming.base_revision + 1,
            incoming_available: conflict.incoming.encrypted_payload.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distance_is_capped_and_counts_a_transposition_as_one() {
        assert_eq!(bounded_distance("hello", "hello", 2), Some(0));
        assert_eq!(bounded_distance("helo", "hello", 2), Some(1));
        assert_eq!(bounded_distance("hlelo", "hello", 2), Some(1));
        assert_eq!(bounded_distance("привет", "превет", 2), Some(1));
        assert_eq!(bounded_distance("привет", "совсем другое", 2), None);
        assert_eq!(bounded_distance("", "abc", 2), None);
        assert_eq!(bounded_distance("", "", 2), Some(0));
        assert_eq!(bounded_distance("ёж", "еж", 1), Some(1));
    }

    #[test]
    fn a_word_list_parses_comments_headers_and_tabs() {
        let words = UserDictionaryPayload::create("проект", Language::Russian, false).unwrap();
        assert_eq!(words.word, "проект");
        let parsed = parse_word_list("# comment\nJARVIS\n\nАЛТРОН\tsome flags\n12345\n");
        assert_eq!(parsed, vec!["JARVIS", "АЛТРОН"]);
        // The same parser is available from the store type for callers that have one.
        assert_eq!(
            UserDictionaryStore::<SqliteSyncRepository, PurposeKeyProvider>::parse_word_list(
                "ёж\n"
            ),
            vec!["ёж"]
        );
    }

    #[test]
    fn a_query_is_normalized_and_bounded() {
        let query = UserDictionaryQuery {
            search: "  ДЖАР ".to_string(),
            limit: 100_000,
            ..UserDictionaryQuery::default()
        }
        .normalized();
        assert_eq!(query.search, "джар");
        assert_eq!(query.limit, 500);
        let default_limit = UserDictionaryQuery::default().normalized();
        assert_eq!(default_limit.limit, 100);
    }
}
