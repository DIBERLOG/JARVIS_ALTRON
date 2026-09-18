//! Vault item payloads, the DTOs the interface receives, and their validation.
//!
//! Everything a user typed — name, username, password, URLs, notes, tags — lives
//! inside the encrypted payload. Only technical metadata stays visible in the
//! database; see `VAULT.md`.
//!
//! The DTO split is deliberate and security relevant:
//!
//! * [`VaultItemSummary`] and [`VaultItemDetails`] never carry the password;
//! * [`SecretRevealResult`] is produced only by an explicit reveal request.
//!
//! Every type here redacts secrets in `Debug`, and the payload zeroizes its
//! buffers on drop.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::sync::SyncError;

/// Version of the JSON payload written into an encrypted vault record.
pub const VAULT_PAYLOAD_SCHEMA_VERSION: u32 = 1;

pub const MAX_NAME_CHARS: usize = 200;
pub const MAX_USERNAME_CHARS: usize = 200;
pub const MAX_PASSWORD_BYTES: usize = 512;
pub const MAX_URLS: usize = 16;
pub const MAX_URL_CHARS: usize = 2048;
pub const MAX_NOTES_BYTES: usize = 64 * 1024;
pub const MAX_TAGS: usize = 32;
pub const MAX_TAG_CHARS: usize = 48;

/// Encrypted content of one vault item.
///
/// The struct zeroizes every field when it is dropped.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize, Zeroize, ZeroizeOnDrop)]
pub struct VaultItemPayload {
    pub schema_version: u32,
    pub name: String,
    pub username: String,
    pub password: String,
    pub urls: Vec<String>,
    pub notes: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

impl fmt::Debug for VaultItemPayload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultItemPayload")
            .field("schema_version", &self.schema_version)
            .field("name", &"<redacted>")
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .field("urls", &self.urls.len())
            .field("notes_len", &self.notes.len())
            .field("tags", &self.tags.len())
            .field("favorite", &self.favorite)
            .field("deleted", &self.deleted_at.is_some())
            .finish()
    }
}

/// A create or edit request coming from the interface.
#[derive(Clone, Deserialize, Serialize)]
pub struct VaultItemDraft {
    pub name: String,
    pub username: String,
    pub password: String,
    pub urls: Vec<String>,
    pub notes: String,
    pub tags: Vec<String>,
    pub favorite: bool,
}

impl fmt::Debug for VaultItemDraft {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultItemDraft")
            .field("name", &"<redacted>")
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .field("urls", &self.urls.len())
            .field("notes_len", &self.notes.len())
            .field("tags", &self.tags.len())
            .field("favorite", &self.favorite)
            .finish()
    }
}

/// Fields that carry no secret: an edit of these never touches the password or
/// the free-form notes.
///
/// This exists so the interface can rename an item, retag it, or change its URL
/// without ever having revealed — and therefore without being able to erase —
/// the stored secret.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VaultMetadataDraft {
    pub name: String,
    pub username: String,
    pub urls: Vec<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
}

impl VaultMetadataDraft {
    /// Metadata part of a full draft.
    pub fn from_draft(draft: &VaultItemDraft) -> Self {
        Self {
            name: draft.name.clone(),
            username: draft.username.clone(),
            urls: draft.urls.clone(),
            tags: draft.tags.clone(),
            favorite: draft.favorite,
        }
    }
}

/// List row: safe fields only, never the password and never the notes.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct VaultItemSummary {
    pub id: Uuid,
    pub revision: u64,
    pub name: String,
    pub username: String,
    /// Host part of the first URL, for recognisability without leaking the path.
    pub url_host: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub has_password: bool,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

impl fmt::Debug for VaultItemSummary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultItemSummary")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("name", &"<redacted>")
            .field("username", &"<redacted>")
            .field("url_host", &"<redacted>")
            .field("favorite", &self.favorite)
            .field("has_password", &self.has_password)
            .field("deleted", &self.deleted_at.is_some())
            .finish()
    }
}

/// Editor view: everything except the password and the free-form notes, which
/// are returned only by [`SecretRevealResult`].
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct VaultItemDetails {
    pub id: Uuid,
    pub revision: u64,
    pub name: String,
    pub username: String,
    pub urls: Vec<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

impl fmt::Debug for VaultItemDetails {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VaultItemDetails")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("name", &"<redacted>")
            .field("username", &"<redacted>")
            .field("urls", &self.urls.len())
            .field("tags", &self.tags.len())
            .field("favorite", &self.favorite)
            .field("deleted", &self.deleted_at.is_some())
            .finish()
    }
}

/// The only type that carries a password to the interface.
///
/// It is produced solely by an explicit reveal command, while the vault is
/// unlocked. `Debug` never prints the secret.
#[derive(Clone, Serialize)]
pub struct SecretRevealResult {
    pub id: Uuid,
    pub revision: u64,
    pub password: String,
    pub notes: String,
    /// Seconds the interface should keep the secret on screen before hiding it
    /// again; the interface may apply its own shorter timeout.
    pub reveal_timeout_seconds: u64,
}

impl fmt::Debug for SecretRevealResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretRevealResult")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("password", &"<redacted>")
            .field("notes", &"<redacted>")
            .finish()
    }
}

/// Which slice of the trash state to include.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultTrashFilter {
    #[default]
    Active,
    Trashed,
    All,
}

/// Ordering for the item list.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultSort {
    #[default]
    NameAsc,
    UpdatedDesc,
    CreatedDesc,
}

/// List request from the interface.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct VaultQuery {
    pub search: String,
    pub tag: Option<String>,
    pub trash: VaultTrashFilter,
    pub favorites_only: bool,
    pub sort: VaultSort,
}

/// Result of a list request.
#[derive(Clone, Debug, Serialize)]
pub struct VaultItemList {
    pub items: Vec<VaultItemSummary>,
    /// Items that could not be decrypted; reported, never hidden.
    pub unreadable: usize,
    pub scanned: usize,
}

/// One side of a vault conflict.
#[derive(Clone, Debug, Serialize)]
pub struct VaultConflictView {
    pub conflict_id: Uuid,
    pub entity_id: Uuid,
    pub is_vault_item: bool,
    /// Name of each side only; no secret is exposed by a conflict listing.
    pub current_name: Option<String>,
    pub incoming_name: Option<String>,
    pub current_revision: u64,
    pub incoming_revision: u64,
}

/// How the operator resolved a conflict.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VaultConflictResolution {
    KeepCurrent,
    AcceptIncoming,
    KeepBoth,
}

/// Result of resolving a conflict.
#[derive(Clone, Debug, Serialize)]
pub struct VaultConflictOutcome {
    pub resolution: VaultConflictResolution,
    pub created_entity_id: Option<Uuid>,
    pub updated_entity_id: Option<Uuid>,
}

/// Counts for the vault status panel.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct VaultStats {
    pub items_total: usize,
    pub items_trashed: usize,
    pub favorites: usize,
    pub conflicts_pending: usize,
    pub unreadable: usize,
}

/// Vault failures. Variants never carry item content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VaultError {
    /// The master key is not available; no secret may be produced.
    StorageLocked,
    /// Records exist but no key file remains; only a backup import can recover.
    KeyMissing,
    NotFound,
    Deleted,
    Unreadable,
    UnsupportedPayloadVersion,
    MalformedPayload,
    NameTooLong,
    NameHasControlCharacters,
    UsernameTooLong,
    UsernameHasControlCharacters,
    PasswordTooLong,
    TooManyUrls,
    UrlTooLong,
    UrlHasControlCharacters,
    NotesTooLarge,
    TooManyTags,
    TagTooLong,
    TagHasControlCharacters,
    InvalidTimestamp,
    /// The generated password request is not satisfiable.
    InvalidPasswordPolicy,
    /// The clipboard could not be read or written.
    ClipboardUnavailable,
    /// Platform clipboard support is missing.
    ClipboardUnsupported,
    /// Re-wrapping the master key failed; old key files remain valid.
    MasterPasswordChangeFailed,
    /// Key files could not be read or written.
    VaultIo,
    /// A failure from the shared key lifecycle (notes storage).
    Notes(crate::notes::NoteError),
    Storage(SyncError),
    Crypto(crate::sync::crypto::CryptoError),
}

impl fmt::Display for VaultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::StorageLocked => "vault storage is locked",
            Self::KeyMissing => "vault storage key is missing",
            Self::NotFound => "vault item not found",
            Self::Deleted => "vault item is deleted",
            Self::Unreadable => "vault item content cannot be read",
            Self::UnsupportedPayloadVersion => "unsupported vault payload version",
            Self::MalformedPayload => "vault payload is malformed",
            Self::NameTooLong => "vault item name is too long",
            Self::NameHasControlCharacters => "vault item name has unsupported characters",
            Self::UsernameTooLong => "vault username is too long",
            Self::UsernameHasControlCharacters => "vault username has unsupported characters",
            Self::PasswordTooLong => "vault password is too long",
            Self::TooManyUrls => "too many URLs",
            Self::UrlTooLong => "URL is too long",
            Self::UrlHasControlCharacters => "URL has unsupported characters",
            Self::NotesTooLarge => "vault notes are too large",
            Self::TooManyTags => "too many tags",
            Self::TagTooLong => "tag is too long",
            Self::TagHasControlCharacters => "tag has unsupported characters",
            Self::InvalidTimestamp => "vault timestamp is invalid",
            Self::InvalidPasswordPolicy => "password policy cannot be satisfied",
            Self::ClipboardUnavailable => "the clipboard is unavailable",
            Self::ClipboardUnsupported => {
                "the clipboard is not supported on this operating system"
            }
            Self::MasterPasswordChangeFailed => "the master password could not be changed",
            Self::VaultIo => "vault key files cannot be read or written",
            Self::Notes(error) => return write!(formatter, "vault key lifecycle error: {error}"),
            Self::Storage(error) => return write!(formatter, "vault storage error: {error}"),
            Self::Crypto(error) => return write!(formatter, "vault crypto error: {error}"),
        })
    }
}

impl std::error::Error for VaultError {}

impl From<SyncError> for VaultError {
    fn from(error: SyncError) -> Self {
        Self::Storage(error)
    }
}

impl From<crate::sync::crypto::CryptoError> for VaultError {
    fn from(error: crate::sync::crypto::CryptoError) -> Self {
        Self::Crypto(error)
    }
}

impl From<crate::notes::NoteError> for VaultError {
    fn from(error: crate::notes::NoteError) -> Self {
        Self::Notes(error)
    }
}

impl VaultItemPayload {
    pub fn create(draft: &VaultItemDraft) -> Result<Self, VaultError> {
        let now = Utc::now().to_rfc3339();
        let payload = Self {
            schema_version: VAULT_PAYLOAD_SCHEMA_VERSION,
            name: draft.name.clone(),
            username: draft.username.clone(),
            password: draft.password.clone(),
            urls: draft.urls.clone(),
            notes: draft.notes.clone(),
            tags: normalize_tags(&draft.tags)?,
            favorite: draft.favorite,
            created_at: now.clone(),
            updated_at: now,
            deleted_at: None,
        };
        payload.validate()?;
        Ok(payload)
    }

    /// Applies an edit, keeping creation time and trash state.
    pub fn apply_draft(&mut self, draft: &VaultItemDraft) -> Result<(), VaultError> {
        self.name = draft.name.clone();
        self.username = draft.username.clone();
        self.password = draft.password.clone();
        self.urls = draft.urls.clone();
        self.notes = draft.notes.clone();
        self.tags = normalize_tags(&draft.tags)?;
        self.favorite = draft.favorite;
        self.validate()?;
        self.touch();
        Ok(())
    }

    /// Applies a metadata-only edit, leaving password and notes untouched.
    pub fn apply_metadata(&mut self, metadata: &VaultMetadataDraft) -> Result<(), VaultError> {
        self.name = metadata.name.clone();
        self.username = metadata.username.clone();
        self.urls = metadata.urls.clone();
        self.tags = normalize_tags(&metadata.tags)?;
        self.favorite = metadata.favorite;
        self.validate()?;
        self.touch();
        Ok(())
    }

    /// Replaces the secret fields only. The caller has revealed them by
    /// definition, so this cannot erase a secret it never saw.
    pub fn apply_secrets(
        &mut self,
        password: &str,
        notes: &str,
    ) -> Result<(), VaultError> {
        self.password = password.to_string();
        self.notes = notes.to_string();
        self.validate()?;
        self.touch();
        Ok(())
    }

    pub fn touch(&mut self) {
        self.updated_at = Utc::now().to_rfc3339();
    }

    pub fn is_trashed(&self) -> bool {
        self.deleted_at.is_some()
    }

    pub fn validate(&self) -> Result<(), VaultError> {
        if self.schema_version != VAULT_PAYLOAD_SCHEMA_VERSION {
            return Err(VaultError::UnsupportedPayloadVersion);
        }
        if self.name.chars().count() > MAX_NAME_CHARS {
            return Err(VaultError::NameTooLong);
        }
        if self.name.contains(['\n', '\r', '\t']) {
            return Err(VaultError::NameHasControlCharacters);
        }
        if self.username.chars().count() > MAX_USERNAME_CHARS {
            return Err(VaultError::UsernameTooLong);
        }
        if self.username.contains(['\n', '\r', '\t']) {
            return Err(VaultError::UsernameHasControlCharacters);
        }
        // A password may contain any character, including spaces; it is never
        // rendered as part of the layout.
        if self.password.len() > MAX_PASSWORD_BYTES {
            return Err(VaultError::PasswordTooLong);
        }
        if self.notes.len() > MAX_NOTES_BYTES {
            return Err(VaultError::NotesTooLarge);
        }
        if self.urls.len() > MAX_URLS {
            return Err(VaultError::TooManyUrls);
        }
        for url in &self.urls {
            if url.chars().count() > MAX_URL_CHARS {
                return Err(VaultError::UrlTooLong);
            }
            if url.contains(['\n', '\r', '\t']) {
                return Err(VaultError::UrlHasControlCharacters);
            }
        }
        if self.tags.len() > MAX_TAGS {
            return Err(VaultError::TooManyTags);
        }
        for tag in &self.tags {
            if tag.chars().count() > MAX_TAG_CHARS {
                return Err(VaultError::TagTooLong);
            }
            if tag.contains(['\n', '\r', '\t']) {
                return Err(VaultError::TagHasControlCharacters);
            }
        }
        if !is_rfc3339(&self.created_at) || !is_rfc3339(&self.updated_at) {
            return Err(VaultError::InvalidTimestamp);
        }
        if let Some(deleted_at) = &self.deleted_at {
            if !is_rfc3339(deleted_at) {
                return Err(VaultError::InvalidTimestamp);
            }
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, VaultError> {
        serde_json::to_vec(self).map_err(|_| VaultError::MalformedPayload)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VaultError> {
        let payload: Self =
            serde_json::from_slice(bytes).map_err(|_| VaultError::MalformedPayload)?;
        payload.validate()?;
        Ok(payload)
    }

    pub fn to_summary(&self, id: Uuid, revision: u64) -> VaultItemSummary {
        VaultItemSummary {
            id,
            revision,
            name: self.name.clone(),
            username: self.username.clone(),
            url_host: self.urls.first().and_then(|url| url_host(url)),
            tags: self.tags.clone(),
            favorite: self.favorite,
            has_password: !self.password.is_empty(),
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            deleted_at: self.deleted_at.clone(),
        }
    }

    pub fn to_details(&self, id: Uuid, revision: u64) -> VaultItemDetails {
        VaultItemDetails {
            id,
            revision,
            name: self.name.clone(),
            username: self.username.clone(),
            urls: self.urls.clone(),
            tags: self.tags.clone(),
            favorite: self.favorite,
            created_at: self.created_at.clone(),
            updated_at: self.updated_at.clone(),
            deleted_at: self.deleted_at.clone(),
        }
    }
}

/// Extracts the host of a URL for display, without scheme, path, or query.
pub fn url_host(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(without_scheme);
    let host = host.rsplit('@').next().unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Trims, de-duplicates (case-insensitively), and validates a tag list.
pub fn normalize_tags(tags: &[String]) -> Result<Vec<String>, VaultError> {
    let mut result: Vec<String> = Vec::new();
    for raw in tags {
        let tag = raw.trim().trim_start_matches('#').trim();
        if tag.is_empty() {
            continue;
        }
        if tag.chars().count() > MAX_TAG_CHARS {
            return Err(VaultError::TagTooLong);
        }
        if tag.contains(['\n', '\r', '\t']) {
            return Err(VaultError::TagHasControlCharacters);
        }
        if result
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(tag))
        {
            continue;
        }
        result.push(tag.to_string());
        if result.len() > MAX_TAGS {
            return Err(VaultError::TooManyTags);
        }
    }
    Ok(result)
}

/// Applies search, tag, favorite, and trash filters, then sorts.
///
/// Returns indices so list rendering never clones secrets.
pub fn matching_indices(items: &[VaultItemPayload], query: &VaultQuery) -> Vec<usize> {
    let needle = query.search.trim().to_lowercase();
    let tag = query
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|tag| !tag.is_empty());
    let mut indices: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            let trash_ok = match query.trash {
                VaultTrashFilter::Active => !item.is_trashed(),
                VaultTrashFilter::Trashed => item.is_trashed(),
                VaultTrashFilter::All => true,
            };
            if !trash_ok {
                return false;
            }
            if query.favorites_only && !item.favorite {
                return false;
            }
            if let Some(tag) = tag {
                if !item
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
            item.name.to_lowercase().contains(&needle)
                || item.username.to_lowercase().contains(&needle)
                || item.notes.to_lowercase().contains(&needle)
                || item
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&needle))
                || item
                    .urls
                    .iter()
                    .any(|url| url.to_lowercase().contains(&needle))
        })
        .map(|(index, _)| index)
        .collect();

    indices.sort_by(|left, right| {
        let left = &items[*left];
        let right = &items[*right];
        let ordering = match query.sort {
            VaultSort::NameAsc => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
            VaultSort::UpdatedDesc => right.updated_at.cmp(&left.updated_at),
            VaultSort::CreatedDesc => right.created_at.cmp(&left.created_at),
        };
        ordering
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.created_at.cmp(&right.created_at))
    });
    indices
}

/// Collects the distinct tag vocabulary, sorted for display.
pub fn collect_tags(items: &[VaultItemPayload]) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for item in items {
        for tag in &item.tags {
            if !tags.iter().any(|existing| existing.eq_ignore_ascii_case(tag)) {
                tags.push(tag.clone());
            }
        }
    }
    tags.sort_by_key(|tag| tag.to_lowercase());
    tags
}

fn is_rfc3339(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
}
