//! Encrypted notes for Windows.
//!
//! Layout:
//!
//! * [`model`] – note and folder payloads, queries, validation;
//! * [`store`] – `NoteStore`, CRUD over encrypted entities;
//! * [`vault`] – master-key lifecycle (initialize, unlock, backup, lock).
//!
//! Plaintext notes never reach SQLite: the payload is JSON-encrypted through the
//! synchronization crypto layer, and only technical metadata stays visible. See
//! `NOTES.md` for the storage model and the in-memory search trade-off.

pub mod model;
pub mod store;
pub mod vault;

#[cfg(test)]
mod tests;

pub use model::{
    collect_tags, excerpt_of, matching_indices, ConflictResolutionOutcome, FolderPayload, Note,
    NoteConflictResolution, NoteConflictView, NoteDraft, NoteError, NoteFolder, NoteList,
    NotePayload, NoteQuery, NoteSort, NoteStats, NoteSummary, TrashFilter,
    FOLDER_PAYLOAD_SCHEMA_VERSION, MAX_BODY_BYTES, MAX_FOLDER_NAME_CHARS, MAX_TAGS, MAX_TAG_CHARS,
    MAX_TITLE_CHARS, NOTE_PAYLOAD_SCHEMA_VERSION,
};
pub use store::{EncryptedNoteStore, NoteStore};
pub use vault::{
    NotesVault, StorageState, StorageStatus, VaultPaths, BACKUP_KEY_FILE, DEVICE_ID_FILE,
    DPAPI_KEY_FILE,
};
