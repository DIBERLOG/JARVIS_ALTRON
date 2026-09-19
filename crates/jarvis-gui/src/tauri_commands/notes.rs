//! Notes commands exposed to the interface.
//!
//! All of them are declared `(async)` so Tauri runs them on a worker thread:
//! SQLite writes, Argon2id derivation, and file dialogs must never block the
//! window thread.
//!
//! Nothing here logs note content: errors carry stable, content-free messages,
//! and titles, bodies, tags, and folder names stay inside the encrypted payload.

use parking_lot::Mutex;
use std::sync::Arc;
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;

use jarvis_core::autocorrect::user_dictionary::EncryptedUserDictionary;
use jarvis_core::autocorrect::AutocorrectError;
use jarvis_core::memory::MemoryError;
use jarvis_core::notes::{
    ConflictResolutionOutcome, Note, NoteConflictResolution, NoteConflictView, NoteDraft,
    NoteError, NoteFolder, NoteList, NoteQuery, NotesVault, StorageStatus,
};
use jarvis_core::vault::{VaultError, VaultSession};

use crate::AppState;

/// Lazily opened encrypted storage shared by every notes and vault command.
///
/// One session owns the database connections and, while unlocked, the master
/// key. Notes and the password vault derive separate working keys from that
/// master key, so sharing the session does not share a key. Cloning the handle
/// shares the same session, which is what the managed application state needs.
#[derive(Clone, Default)]
pub struct NotesHandle {
    session: Arc<Mutex<Option<VaultSession>>>,
}

impl NotesHandle {
    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
        }
    }

    /// Opens the storage during application start.
    ///
    /// Doing this eagerly means the storage status is ready for the first notes
    /// page and key-file problems surface in the log immediately. A failure is
    /// only logged: the interface still shows the storage gate, and every
    /// command retries the open on demand.
    pub fn preload(&self) {
        let mut guard = self.session.lock();
        if guard.is_some() {
            return;
        }
        match VaultSession::open_production() {
            Ok(session) => *guard = Some(session),
            Err(error) => log::warn!("storage: unavailable at startup: {}", error),
        }
    }

    /// Runs `action` against the shared key lifecycle, opening the production
    /// storage on first use. The lock is held for the whole operation so
    /// commands serialize.
    pub fn with<T>(
        &self,
        action: impl FnOnce(&mut NotesVault) -> Result<T, NoteError>,
    ) -> Result<T, String> {
        let mut guard = self.session.lock();
        if guard.is_none() {
            *guard = Some(VaultSession::open_production().map_err(describe_vault)?);
        }
        let session = guard
            .as_mut()
            .ok_or_else(|| describe_vault(VaultError::StorageLocked))?;
        session.with_storage(action).map_err(describe)
    }

    /// Runs `action` against the shared session, for vault commands.
    pub fn with_session<T>(
        &self,
        action: impl FnOnce(&mut VaultSession) -> Result<T, VaultError>,
    ) -> Result<T, String> {
        let mut guard = self.session.lock();
        if guard.is_none() {
            *guard = Some(VaultSession::open_production().map_err(describe_vault)?);
        }
        let session = guard
            .as_mut()
            .ok_or_else(|| describe_vault(VaultError::StorageLocked))?;
        action(session).map_err(describe_vault)
    }

    /// Runs `action` against the shared session, for AI-memory commands.
    ///
    /// The memory layer has its own error type, so it gets its own entry point
    /// instead of being folded into the vault error path. The shared session is what
    /// keeps one master key, and therefore one unlock state, behind all three
    /// encrypted stores.
    pub fn with_memory<T>(
        &self,
        action: impl FnOnce(&mut VaultSession) -> Result<T, MemoryError>,
    ) -> Result<T, String> {
        let mut guard = self.session.lock();
        if guard.is_none() {
            *guard = Some(VaultSession::open_production().map_err(describe_vault)?);
        }
        let session = guard
            .as_mut()
            .ok_or_else(|| describe_vault(VaultError::StorageLocked))?;
        action(session).map_err(describe_memory)
    }

    /// Runs `action` against the user's own encrypted word list.
    ///
    /// The spelling layer has its own error type and its own derived key, so it gets its
    /// own entry point. The shared session is what keeps one master key, and therefore one
    /// unlock state, behind all four encrypted stores — while the word list stays a
    /// different database opened with a different derived key.
    pub fn with_autocorrect<T>(
        &self,
        action: impl FnOnce(&mut EncryptedUserDictionary) -> Result<T, AutocorrectError>,
    ) -> Result<T, String> {
        let mut guard = self.session.lock();
        if guard.is_none() {
            *guard = Some(VaultSession::open_production().map_err(describe_vault)?);
        }
        let session = guard
            .as_mut()
            .ok_or_else(|| describe_vault(VaultError::StorageLocked))?;
        let store = session.autocorrect_store().map_err(describe_autocorrect)?;
        action(store).map_err(describe_autocorrect)
    }

    /// Whether the shared encrypted storage is currently unlocked.
    pub fn is_unlocked(&self) -> bool {
        let guard = self.session.lock();
        guard
            .as_ref()
            .map(|session| session.is_unlocked())
            .unwrap_or(false)
    }

    /// Locks the shared storage, dropping the master key and every derived key.
    pub fn lock(&self) {
        let mut guard = self.session.lock();
        if let Some(session) = guard.as_mut() {
            session.lock();
        }
    }

    /// Forgets the opened storage, for example when its directory is
    /// unreachable.
    pub fn reset(&self) {
        *self.session.lock() = None;
    }
}

/// Whether an error is simply "the storage is locked".
///
/// Locked is the normal state of the encrypted storage: it is locked at start-up,
/// it is locked by the idle timer, it is locked on an explicit lock, and it is
/// locked after every exit. A command that arrives while it is locked — a poll
/// from an open page, a button pressed before the password was typed — is not a
/// fault, and logging it as one buries the real failures in noise. Everything
/// else stays a warning.
fn is_locked(error: &impl std::fmt::Display) -> bool {
    let message = error.to_string().to_lowercase();
    message.contains("storage is locked")
        || message.contains("storage_locked")
        || message.contains("storage locked")
}

/// Logs a storage failure at the level it deserves, and returns the message.
fn log_storage_error(prefix: &str, error: &impl std::fmt::Display) -> String {
    let message = error.to_string();
    if is_locked(error) {
        // The expected state, not a fault: the interface asks for the password.
        log::debug!("{prefix}: storage is locked");
    } else {
        log::warn!("{prefix}: {message}");
    }
    message
}

/// Turns a vault error into a message safe to show and to log.
fn describe_vault(error: VaultError) -> String {
    log_storage_error("vault", &error)
}

/// Turns a notes error into a message safe to show and to log.
fn describe(error: NoteError) -> String {
    // Only content-free messages are logged; note text never reaches a log line.
    log_storage_error("notes", &error)
}

/// Turns an AI-memory error into a message safe to show and to log.
///
/// A memory error never carries stored text: the secret variants name the kinds
/// that were recognized and nothing else, so this line cannot leak memory.
fn describe_memory(error: MemoryError) -> String {
    log_storage_error("memory", &error)
}

/// Turns an autocorrect error into a message safe to show and to log.
///
/// The message never carries document text, a matched secret, or a key.
fn describe_autocorrect(error: AutocorrectError) -> String {
    log_storage_error("autocorrect", &error)
}

// ------------------------------------------------------------------ storage

#[tauri::command(async)]
pub fn notes_status(state: tauri::State<'_, AppState>) -> Result<StorageStatus, String> {
    state.notes.with(|vault| vault.status())
}

#[tauri::command(async)]
pub fn notes_initialize(
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<StorageStatus, String> {
    state.notes.with(|vault| vault.initialize(&password))
}

#[tauri::command(async)]
pub fn notes_unlock_dpapi(state: tauri::State<'_, AppState>) -> Result<StorageStatus, String> {
    state.notes.with(|vault| vault.unlock_with_dpapi())
}

#[tauri::command(async)]
pub fn notes_unlock_password(
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<StorageStatus, String> {
    state
        .notes
        .with(|vault| vault.unlock_with_password(&password))
}

#[tauri::command(async)]
pub fn notes_lock(state: tauri::State<'_, AppState>) -> Result<StorageStatus, String> {
    // The undo journal holds the text of the documents it corrected, so it is dropped
    // with the key it was used under.
    state.autocorrect.clear_journals();
    state.notes.with(|vault| {
        vault.lock();
        vault.status()
    })
}

#[tauri::command(async)]
pub fn notes_import_backup(
    state: tauri::State<'_, AppState>,
    envelope: String,
    password: String,
) -> Result<StorageStatus, String> {
    state
        .notes
        .with(|vault| vault.import_backup(&envelope, &password))
}

/// Builds a fresh portable envelope for the in-memory master key.
#[tauri::command(async)]
pub fn notes_export_backup(
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<String, String> {
    state
        .notes
        .with(|vault| vault.export_backup_json(&password))
}

/// Writes a portable envelope to a user-chosen file.
///
/// Returns the path, or an empty string when the user cancels.
#[tauri::command(async)]
pub fn notes_export_backup_file(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    password: String,
) -> Result<String, String> {
    let Some(destination) = save_path(&app, "jarvis-key-backup.json", "JSON", "json") else {
        return Ok(String::new());
    };
    state.notes.with(|vault| {
        vault
            .export_backup_to(&password, &destination)
            .map(|path| path.display().to_string())
    })
}

/// Reads a portable envelope from a user-chosen file and imports it.
///
/// Cancelling the picker is not an error: the current status is returned
/// unchanged.
#[tauri::command(async)]
pub fn notes_import_backup_file(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    password: String,
) -> Result<StorageStatus, String> {
    let Some(source) = open_path(&app) else {
        return state.notes.with(|vault| vault.status());
    };
    let envelope = std::fs::read_to_string(&source).map_err(|_| describe(NoteError::VaultIo))?;
    state
        .notes
        .with(|vault| vault.import_backup(&envelope, &password))
}

// --------------------------------------------------------------------- notes

#[tauri::command(async)]
pub fn notes_list(state: tauri::State<'_, AppState>, query: NoteQuery) -> Result<NoteList, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.list_notes(&query)))
}

#[tauri::command(async)]
pub fn notes_get(state: tauri::State<'_, AppState>, id: Uuid) -> Result<Option<Note>, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.get_note(id)))
}

#[tauri::command(async)]
pub fn notes_create(state: tauri::State<'_, AppState>, draft: NoteDraft) -> Result<Note, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.create_note(&draft)))
}

#[tauri::command(async)]
pub fn notes_update(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    draft: NoteDraft,
) -> Result<Note, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.update_note(id, &draft)))
}

/// Autosave endpoint: identical to an update, kept separate so the interface
/// can distinguish an explicit save from a debounced one.
#[tauri::command(async)]
pub fn notes_autosave(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    draft: NoteDraft,
) -> Result<Note, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.update_note(id, &draft)))
}

#[tauri::command(async)]
pub fn notes_set_pinned(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    pinned: bool,
) -> Result<Note, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.set_pinned(id, pinned)))
}

#[tauri::command(async)]
pub fn notes_trash(state: tauri::State<'_, AppState>, id: Uuid) -> Result<Note, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.trash_note(id)))
}

#[tauri::command(async)]
pub fn notes_restore(state: tauri::State<'_, AppState>, id: Uuid) -> Result<Note, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.restore_note(id)))
}

#[tauri::command(async)]
pub fn notes_purge(state: tauri::State<'_, AppState>, id: Uuid) -> Result<(), String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.purge_note(id)))
}

// ------------------------------------------------------------------- folders

#[tauri::command(async)]
pub fn notes_folders(state: tauri::State<'_, AppState>) -> Result<Vec<NoteFolder>, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.folders()))
}

#[tauri::command(async)]
pub fn notes_create_folder(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<NoteFolder, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.create_folder(&name)))
}

#[tauri::command(async)]
pub fn notes_rename_folder(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    name: String,
) -> Result<NoteFolder, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.rename_folder(id, &name)))
}

#[tauri::command(async)]
pub fn notes_trash_folder(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<NoteFolder, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.trash_folder(id)))
}

#[tauri::command(async)]
pub fn notes_restore_folder(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<NoteFolder, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.restore_folder(id)))
}

#[tauri::command(async)]
pub fn notes_purge_folder(state: tauri::State<'_, AppState>, id: Uuid) -> Result<(), String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.purge_folder(id)))
}

#[tauri::command(async)]
pub fn notes_tags(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.all_tags()))
}

// ----------------------------------------------------------------- conflicts

#[tauri::command(async)]
pub fn notes_conflicts(state: tauri::State<'_, AppState>) -> Result<Vec<NoteConflictView>, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.conflicts()))
}

#[tauri::command(async)]
pub fn notes_resolve_conflict(
    state: tauri::State<'_, AppState>,
    conflict: Uuid,
    resolution: NoteConflictResolution,
) -> Result<ConflictResolutionOutcome, String> {
    state
        .notes
        .with(|vault| vault.with_store(|store| store.resolve_conflict(conflict, resolution)))
}

// ------------------------------------------------------------------- helpers

fn save_path(
    app: &tauri::AppHandle,
    file_name: &str,
    filter_name: &str,
    extension: &str,
) -> Option<std::path::PathBuf> {
    app.dialog()
        .file()
        .set_title("JARVIS")
        .set_file_name(file_name)
        .add_filter(filter_name, &[extension])
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
}

fn open_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.dialog()
        .file()
        .set_title("JARVIS")
        .add_filter("JSON", &["json"])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The locked state of the encrypted storage is normal, not a fault.
    ///
    /// It is locked at start-up, on the idle timer, on an explicit lock, and
    /// after every exit, so a command that arrives while it is locked — a poll
    /// from an open page, a button pressed before the password was typed — must
    /// not produce a warning that buries the real failures.
    #[test]
    fn a_locked_storage_is_not_a_warning() {
        assert!(is_locked(&VaultError::StorageLocked));
        assert!(is_locked(&NoteError::StorageLocked));
        assert!(is_locked(&MemoryError::StorageLocked));
        assert!(is_locked(&AutocorrectError::StorageLocked));
    }

    /// Everything else stays a warning: a damaged document, a missing key, a
    /// storage that cannot be opened.
    #[test]
    fn every_other_failure_still_warns() {
        assert!(!is_locked(&VaultError::KeyMissing));
        assert!(!is_locked(&VaultError::Unreadable));
        assert!(!is_locked(&NoteError::Unreadable));
        // The message is returned unchanged either way: the interface shows it.
        let message = log_storage_error("vault", &VaultError::StorageLocked);
        assert!(message.contains("locked"), "{message}");
        let message = log_storage_error("vault", &VaultError::Unreadable);
        assert!(!message.is_empty());
    }
}
