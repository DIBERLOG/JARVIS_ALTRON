//! Encrypted password vault for Windows.
//!
//! Layout:
//!
//! * [`model`] – vault item payloads and the DTOs the interface receives;
//! * [`store`] – `VaultStore`, encrypted password storage over the sync layer;
//! * [`session`] – `VaultSession`, the shared master-key session plus the vault
//!   database and its derived key;
//! * [`keys`] – master-password change;
//! * [`generator`] – CSPRNG password generation;
//! * [`clipboard`] – timed clipboard copy with automatic clearing.
//!
//! Two rules shape the module:
//!
//! 1. The vault never touches the master key. It derives `JARVIS/vault/v1` from
//!    it through [`PurposeKeyProvider`], so a compromise of the vault domain does
//!    not yield the notes key or the master key.
//! 2. Secrets cross the process boundary only through an explicit reveal command
//!    ([`SecretRevealResult`]); lists and details never contain a password.
//!
//! See `VAULT.md` for the threat model and the honest limits of this stage.

pub mod clipboard;
pub mod generator;
pub mod keys;
pub mod model;
pub mod session;
pub mod store;

#[cfg(test)]
mod tests;

pub use clipboard::{
    clamp_clear_seconds, ClipboardBackend, ClipboardError, ClipboardGuard, ClipboardOutcome,
    ClipboardStatus, SystemClipboard, DEFAULT_CLEAR_SECONDS, MAX_CLEAR_SECONDS,
    MIN_CLEAR_SECONDS,
};
pub use generator::{
    estimate_entropy_bits, generate_password, generate_password_from, PasswordPolicy,
    DEFAULT_LENGTH, MAX_LENGTH, MIN_LENGTH,
};
pub use keys::{change_master_password, MasterPasswordChange};
pub use model::{
    collect_tags, matching_indices, normalize_tags, url_host, SecretRevealResult,
    VaultConflictOutcome, VaultConflictResolution, VaultConflictView, VaultError, VaultItemDetails,
    VaultItemDraft, VaultItemList, VaultItemPayload, VaultItemSummary, VaultMetadataDraft,
    VaultQuery, VaultSort, VaultStats, VaultTrashFilter, MAX_NAME_CHARS, MAX_NOTES_BYTES,
    MAX_PASSWORD_BYTES, MAX_TAGS, MAX_TAG_CHARS, MAX_URLS, MAX_URL_CHARS, MAX_USERNAME_CHARS,
    VAULT_PAYLOAD_SCHEMA_VERSION,
};
pub use session::{
    normalize_timeout, IdleLock, VaultResult, VaultSession, VaultStatus, DEFAULT_IDLE_TIMEOUT_SECONDS,
    IDLE_TIMEOUT_OPTIONS, VAULT_DB_FILE,
};
pub use store::{EncryptedVaultStore, VaultStore, DEFAULT_REVEAL_TIMEOUT_SECONDS};
