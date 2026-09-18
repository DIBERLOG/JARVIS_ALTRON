//! Note and folder payloads, query shapes, and their validation.
//!
//! Title, body, folder names, and tags are sensitive and live **inside** the
//! encrypted payload. Only technical metadata (entity ID, entity type, revision,
//! journal cursor, tombstone flag, payload schema version) is stored in the
//! clear by the synchronization layer.
//!
//! Every type here redacts content in its `Debug` implementation, so accidental
//! diagnostic output cannot leak a note.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

/// Version of the JSON payload written into an encrypted note record.
pub const NOTE_PAYLOAD_SCHEMA_VERSION: u32 = 1;
/// Version of the JSON payload written into an encrypted folder record.
pub const FOLDER_PAYLOAD_SCHEMA_VERSION: u32 = 1;

pub const MAX_TITLE_CHARS: usize = 300;
pub const MAX_BODY_BYTES: usize = 256 * 1024;
pub const MAX_TAGS: usize = 32;
pub const MAX_TAG_CHARS: usize = 48;
pub const MAX_FOLDER_NAME_CHARS: usize = 120;
pub const EXCERPT_CHARS: usize = 160;

/// Encrypted content of one note. Serialized to JSON before encryption.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct NotePayload {
    pub schema_version: u32,
    pub title: String,
    pub body: String,
    pub folder_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Set while the note sits in the trash; `None` means it is active.
    pub deleted_at: Option<String>,
}

impl fmt::Debug for NotePayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotePayload")
            .field("schema_version", &self.schema_version)
            .field("title", &"<redacted>")
            .field("body_len", &self.body.len())
            .field("folder_id", &self.folder_id)
            .field("tags", &self.tags.len())
            .field("pinned", &self.pinned)
            .field("deleted", &self.deleted_at.is_some())
            .finish()
    }
}

/// A validated note creation or edit request.
#[derive(Clone, Deserialize, Serialize)]
pub struct NoteDraft {
    pub title: String,
    pub body: String,
    pub folder_id: Option<Uuid>,
    pub tags: Vec<String>,
}

impl fmt::Debug for NoteDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NoteDraft")
            .field("title", &"<redacted>")
            .field("body_len", &self.body.len())
            .field("folder_id", &self.folder_id)
            .field("tags", &self.tags.len())
            .finish()
    }
}

impl NotePayload {
    /// Builds a payload for a new note.
    pub fn create(draft: &NoteDraft) -> Result<Self, NoteError> {
        let now = Utc::now().to_rfc3339();
        let payload = Self {
            schema_version: NOTE_PAYLOAD_SCHEMA_VERSION,
            title: draft.title.clone(),
            body: draft.body.clone(),
            folder_id: draft.folder_id,
            tags: normalize_tags(&draft.tags)?,
            pinned: false,
            created_at: now.clone(),
            updated_at: now,
            deleted_at: None,
        };
        payload.validate()?;
        Ok(payload)
    }

    /// Applies an edit, keeping creation time, pin state, and trash state.
    pub fn apply_draft(&mut self, draft: &NoteDraft) -> Result<(), NoteError> {
        let content_changed = self.title != draft.title
            || self.body != draft.body
            || self.folder_id != draft.folder_id
            || self.tags != draft.tags;
        self.title = draft.title.clone();
        self.body = draft.body.clone();
        self.folder_id = draft.folder_id;
        self.tags = normalize_tags(&draft.tags)?;
        self.validate()?;
        if content_changed {
            self.touch();
        }
        Ok(())
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now().to_rfc3339();
    }

    pub fn is_trashed(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Rejects payloads that would corrupt the UI or exceed storage limits.
    pub fn validate(&self) -> Result<(), NoteError> {
        if self.schema_version != NOTE_PAYLOAD_SCHEMA_VERSION {
            return Err(NoteError::UnsupportedPayloadVersion);
        }
        if self.title.chars().count() > MAX_TITLE_CHARS {
            return Err(NoteError::TitleTooLong);
        }
        if self.title.contains(['\n', '\r', '\t']) {
            return Err(NoteError::TitleHasControlCharacters);
        }
        if self.body.len() > MAX_BODY_BYTES {
            return Err(NoteError::BodyTooLarge);
        }
        if self.tags.len() > MAX_TAGS {
            return Err(NoteError::TooManyTags);
        }
        for tag in &self.tags {
            if tag.chars().count() > MAX_TAG_CHARS {
                return Err(NoteError::TagTooLong);
            }
            if tag.contains(['\n', '\r', '\t']) {
                return Err(NoteError::TagHasControlCharacters);
            }
        }
        if !is_rfc3339(&self.created_at) || !is_rfc3339(&self.updated_at) {
            return Err(NoteError::InvalidTimestamp);
        }
        if let Some(deleted_at) = &self.deleted_at {
            if !is_rfc3339(deleted_at) {
                return Err(NoteError::InvalidTimestamp);
            }
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, NoteError> {
        serde_json::to_vec(self).map_err(|_| NoteError::MalformedPayload)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NoteError> {
        let payload: Self =
            serde_json::from_slice(bytes).map_err(|_| NoteError::MalformedPayload)?;
        payload.validate()?;
        Ok(payload)
    }
}

/// Encrypted content of one folder.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct FolderPayload {
    pub schema_version: u32,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

impl fmt::Debug for FolderPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FolderPayload")
            .field("schema_version", &self.schema_version)
            .field("name", &"<redacted>")
            .field("deleted", &self.deleted_at.is_some())
            .finish()
    }
}

impl FolderPayload {
    pub fn create(name: &str) -> Result<Self, NoteError> {
        let name = normalize_folder_name(name)?;
        let now = Utc::now().to_rfc3339();
        Ok(Self {
            schema_version: FOLDER_PAYLOAD_SCHEMA_VERSION,
            name,
            created_at: now.clone(),
            updated_at: now,
            deleted_at: None,
        })
    }

    pub fn rename(&mut self, name: &str) -> Result<(), NoteError> {
        let name = normalize_folder_name(name)?;
        if self.name != name {
            self.name = name;
            self.updated_at = Utc::now().to_rfc3339();
        }
        Ok(())
    }

    pub fn is_trashed(&self) -> bool {
        self.deleted_at.is_some()
    }

    pub fn validate(&self) -> Result<(), NoteError> {
        if self.schema_version != FOLDER_PAYLOAD_SCHEMA_VERSION {
            return Err(NoteError::UnsupportedPayloadVersion);
        }
        let name = normalize_folder_name(&self.name)?;
        if name != self.name {
            return Err(NoteError::MalformedPayload);
        }
        if !is_rfc3339(&self.created_at) || !is_rfc3339(&self.updated_at) {
            return Err(NoteError::InvalidTimestamp);
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, NoteError> {
        serde_json::to_vec(self).map_err(|_| NoteError::MalformedPayload)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NoteError> {
        let payload: Self =
            serde_json::from_slice(bytes).map_err(|_| NoteError::MalformedPayload)?;
        payload.validate()?;
        Ok(payload)
    }
}

/// A note as delivered to the interface.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct Note {
    pub id: Uuid,
    pub revision: u64,
    pub title: String,
    pub body: String,
    pub folder_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

impl fmt::Debug for Note {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Note")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("title", &"<redacted>")
            .field("body_len", &self.body.len())
            .field("tags", &self.tags.len())
            .field("pinned", &self.pinned)
            .field("deleted", &self.deleted_at.is_some())
            .finish()
    }
}

impl Note {
    pub fn from_payload(id: Uuid, revision: u64, payload: NotePayload) -> Self {
        Self {
            id,
            revision,
            title: payload.title,
            body: payload.body,
            folder_id: payload.folder_id,
            tags: payload.tags,
            pinned: payload.pinned,
            created_at: payload.created_at,
            updated_at: payload.updated_at,
            deleted_at: payload.deleted_at,
        }
    }

    pub fn into_payload(self) -> NotePayload {
        NotePayload {
            schema_version: NOTE_PAYLOAD_SCHEMA_VERSION,
            title: self.title,
            body: self.body,
            folder_id: self.folder_id,
            tags: self.tags,
            pinned: self.pinned,
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        }
    }

    pub fn is_trashed(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Single-line preview for list rows.
    pub fn excerpt(&self) -> String {
        excerpt_of(&self.body)
    }

    pub fn to_summary(&self) -> NoteSummary {
        NoteSummary {
            id: self.id,
            revision: self.revision,
            title: self.title.clone(),
            excerpt: self.excerpt(),
            folder_id: self.folder_id,
            tags: self.tags.clone(),
            pinned: self.pinned,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            deleted_at: self.deleted_at.clone(),
        }
    }
}

/// List-row projection of a note: everything except the full body.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NoteSummary {
    pub id: Uuid,
    pub revision: u64,
    pub title: String,
    pub excerpt: String,
    pub folder_id: Option<Uuid>,
    pub tags: Vec<String>,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

impl fmt::Debug for NoteSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NoteSummary")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("title", &"<redacted>")
            .field("excerpt", &"<redacted>")
            .field("pinned", &self.pinned)
            .field("deleted", &self.deleted_at.is_some())
            .finish()
    }
}

/// A folder as delivered to the interface.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct NoteFolder {
    pub id: Uuid,
    pub revision: u64,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

impl fmt::Debug for NoteFolder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NoteFolder")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("name", &"<redacted>")
            .field("deleted", &self.deleted_at.is_some())
            .finish()
    }
}

impl NoteFolder {
    pub fn is_trashed(&self) -> bool {
        self.deleted_at.is_some()
    }
}

/// Which slice of the trash state to include.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashFilter {
    /// Active notes only.
    #[default]
    Active,
    /// Notes in the trash only.
    Trashed,
    /// Both.
    All,
}

/// Ordering for the note list.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteSort {
    #[default]
    UpdatedDesc,
    UpdatedAsc,
    CreatedDesc,
    CreatedAsc,
    TitleAsc,
}

/// List request from the interface.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct NoteQuery {
    pub search: String,
    pub folder_id: Option<Uuid>,
    pub tag: Option<String>,
    pub trash: TrashFilter,
    /// Keep pinned notes at the top of the page.
    pub pinned_first: bool,
    pub sort: NoteSort,
}

/// Result of a list request.
#[derive(Clone, Debug, Serialize)]
pub struct NoteList {
    pub items: Vec<NoteSummary>,
    /// Active notes that could not be decrypted or parsed, if any.
    pub unreadable: usize,
    /// Total number of notes scanned, before filtering.
    pub scanned: usize,
}

/// One side of a note conflict, if its payload can be read.
#[derive(Clone, Debug, Serialize)]
pub struct NoteConflictView {
    pub conflict_id: Uuid,
    pub entity_id: Uuid,
    pub entity_type: crate::sync::SyncEntityType,
    pub is_note: bool,
    pub current: Option<Note>,
    pub incoming: Option<Note>,
    pub current_revision: u64,
    pub incoming_revision: u64,
}

/// How the operator resolved a conflict.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteConflictResolution {
    /// Keep the stored version and dismiss the incoming one.
    KeepCurrent,
    /// Replace the stored version with the incoming one.
    AcceptIncoming,
    /// Store the incoming version as a second, independent note.
    KeepBoth,
}

/// Result of resolving a conflict.
#[derive(Clone, Debug, Serialize)]
pub struct ConflictResolutionOutcome {
    pub resolution: NoteConflictResolution,
    /// Set when a new entity was created by `KeepBoth`.
    pub created_entity_id: Option<Uuid>,
    /// Set when an existing entity was updated by `AcceptIncoming`.
    pub updated_entity_id: Option<Uuid>,
}

/// Counts used by the storage status.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct NoteStats {
    pub notes_total: usize,
    pub notes_trashed: usize,
    pub folders_total: usize,
    pub conflicts_pending: usize,
    pub unreadable: usize,
}

/// Errors from the notes layer. Variants never carry note content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NoteError {
    /// The master key is not available; no note content may be produced.
    StorageLocked,
    /// No key material exists yet; the storage must be initialized first.
    NotInitialized,
    /// Key material already exists; create is refused to avoid orphaning data.
    AlreadyInitialized,
    /// Entities exist but no key file remains: only a backup import can help.
    KeyMissing,
    /// A note, folder, or conflict with that identifier does not exist.
    NotFound,
    /// A deleted entity cannot be edited.
    Deleted,
    /// The stored payload could not be decrypted or parsed.
    Unreadable,
    UnsupportedPayloadVersion,
    MalformedPayload,
    TitleTooLong,
    TitleHasControlCharacters,
    BodyTooLarge,
    TooManyTags,
    TagTooLong,
    TagHasControlCharacters,
    EmptyFolderName,
    FolderNameTooLong,
    FolderNameHasControlCharacters,
    InvalidTimestamp,
    /// A conflict's incoming side could not be decrypted.
    ConflictUnreadable,
    /// Key files could not be read or written.
    VaultIo,
    /// The platform cannot protect the local key.
    PlatformUnsupported,
    Storage(crate::sync::SyncError),
    Crypto(crate::sync::crypto::CryptoError),
}

impl fmt::Display for NoteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::StorageLocked => "notes storage is locked",
            Self::NotInitialized => "notes storage is not initialized",
            Self::AlreadyInitialized => "notes storage is already initialized",
            Self::KeyMissing => "notes storage key is missing",
            Self::NotFound => "note not found",
            Self::Deleted => "note is deleted",
            Self::Unreadable => "note content cannot be read",
            Self::UnsupportedPayloadVersion => "unsupported note payload version",
            Self::MalformedPayload => "note payload is malformed",
            Self::TitleTooLong => "note title is too long",
            Self::TitleHasControlCharacters => "note title has unsupported characters",
            Self::BodyTooLarge => "note body is too large",
            Self::TooManyTags => "too many tags",
            Self::TagTooLong => "tag is too long",
            Self::TagHasControlCharacters => "tag has unsupported characters",
            Self::EmptyFolderName => "folder name is empty",
            Self::FolderNameTooLong => "folder name is too long",
            Self::FolderNameHasControlCharacters => "folder name has unsupported characters",
            Self::InvalidTimestamp => "note timestamp is invalid",
            Self::ConflictUnreadable => "conflicting version cannot be read",
            Self::VaultIo => "key files cannot be read or written",
            Self::PlatformUnsupported => "platform key protection is unavailable",
            Self::Storage(error) => return write!(formatter, "notes storage error: {error}"),
            Self::Crypto(error) => return write!(formatter, "notes crypto error: {error}"),
        })
    }
}

impl std::error::Error for NoteError {}

impl From<crate::sync::SyncError> for NoteError {
    fn from(error: crate::sync::SyncError) -> Self {
        Self::Storage(error)
    }
}

impl From<crate::sync::crypto::CryptoError> for NoteError {
    fn from(error: crate::sync::crypto::CryptoError) -> Self {
        Self::Crypto(error)
    }
}

/// Trims, de-duplicates (case-insensitively), and validates a tag list.
pub fn normalize_tags(tags: &[String]) -> Result<Vec<String>, NoteError> {
    let mut result: Vec<String> = Vec::new();
    for raw in tags {
        let tag = raw.trim().trim_start_matches('#').trim();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > MAX_TAG_CHARS {
            return Err(NoteError::TagTooLong);
        }
        if tag.contains(['\n', '\r', '\t']) {
            return Err(NoteError::TagHasControlCharacters);
        }
        if result
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(tag))
        {
            continue;
        }
        result.push(tag.to_string());
        if result.len() > MAX_TAGS {
            return Err(NoteError::TooManyTags);
        }
    }
    Ok(result)
}

fn normalize_folder_name(name: &str) -> Result<String, NoteError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(NoteError::EmptyFolderName);
    }
    if name.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(NoteError::FolderNameTooLong);
    }
    if name.contains(['\n', '\r', '\t']) {
        return Err(NoteError::FolderNameHasControlCharacters);
    }
    Ok(name.to_string())
}

/// Collapses whitespace and truncates a body for single-line display.
pub fn excerpt_of(body: &str) -> String {
    let mut excerpt = String::new();
    let mut pending_space = false;
    for character in body.chars() {
        if character.is_whitespace() {
            pending_space = !excerpt.is_empty();
            continue;
        }
        if pending_space {
            excerpt.push(' ');
            pending_space = false;
        }
        excerpt.push(character);
        if excerpt.chars().count() > EXCERPT_CHARS {
            break;
        }
    }
    if excerpt.chars().count() > EXCERPT_CHARS {
        let mut truncated: String = excerpt.chars().take(EXCERPT_CHARS - 1).collect();
        truncated.push('…');
        return truncated;
    }
    excerpt
}

fn is_rfc3339(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
}

/// Applies search, folder/tag, and trash filters, then sorts.
///
/// Returns indices into `notes` so list rendering never clones note bodies.
pub fn matching_indices(notes: &[Note], query: &NoteQuery) -> Vec<usize> {
    let needle = query.search.trim().to_lowercase();
    let tag = query
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|tag| !tag.is_empty());
    let mut indices: Vec<usize> = notes
        .iter()
        .enumerate()
        .filter(|(_, note)| {
            let trash_ok = match query.trash {
                TrashFilter::Active => !note.is_trashed(),
                TrashFilter::Trashed => note.is_trashed(),
                TrashFilter::All => true,
            };
            if !trash_ok {
                return false;
            }
            if let Some(folder_id) = query.folder_id {
                if note.folder_id != Some(folder_id) {
                    return false;
                }
            }
            if let Some(tag) = tag {
                if !note
                    .tags
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(tag))
                {
                    return false;
                }
            }
            if needle.is_empty() {
                return true;
            }
            note.title.to_lowercase().contains(&needle)
                || note.body.to_lowercase().contains(&needle)
                || note
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&needle))
        })
        .map(|(index, _)| index)
        .collect();

    indices.sort_by(|left, right| {
        let left = &notes[*left];
        let right = &notes[*right];
        if query.pinned_first && left.pinned != right.pinned {
            return right.pinned.cmp(&left.pinned);
        }
        let ordering = match query.sort {
            NoteSort::UpdatedDesc => right.updated_at.cmp(&left.updated_at),
            NoteSort::UpdatedAsc => left.updated_at.cmp(&right.updated_at),
            NoteSort::CreatedDesc => right.created_at.cmp(&left.created_at),
            NoteSort::CreatedAsc => left.created_at.cmp(&right.created_at),
            NoteSort::TitleAsc => left
                .title
                .to_lowercase()
                .cmp(&right.title.to_lowercase()),
        };
        // Deterministic tie-break so paging and tests are stable.
        ordering.then_with(|| left.id.cmp(&right.id))
    });
    indices
}


/// Collects the distinct tag vocabulary from notes, sorted for display.
pub fn collect_tags(notes: &[Note]) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for note in notes {
        for tag in &note.tags {
            if !tags.iter().any(|existing| existing.eq_ignore_ascii_case(tag)) {
                tags.push(tag.clone());
            }
        }
    }
    tags.sort_by_key(|tag| tag.to_lowercase());
    tags
}
