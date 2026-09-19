//! Password vault commands exposed to the interface.
//!
//! Rules that shape this file:
//!
//! * every command runs on a worker thread (`#[tauri::command(async)]`), so
//!   Argon2id, SQLite, and file dialogs never block the window;
//! * a password only ever leaves Rust through an explicit reveal command;
//! * copying a secret happens entirely in Rust — the command returns clipboard
//!   state, never the copied value;
//! * an idle timeout is enforced here as well as in the interface, so the master
//!   key is dropped even when the page stops running its own timer;
//! * nothing logs a name, username, password, URL, tag, or note.

use parking_lot::Mutex;
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri_plugin_dialog::DialogExt;
use uuid::Uuid;
use zeroize::Zeroizing;

use jarvis_core::vault::{
    generate_password, ClipboardGuard, ClipboardStatus, EncryptedVaultStore, IdleLock,
    MasterPasswordChange, PasswordPolicy, SecretRevealResult, SystemClipboard,
    VaultConflictOutcome, VaultConflictResolution, VaultConflictView, VaultError, VaultItemDetails,
    VaultItemDraft, VaultItemList, VaultMetadataDraft, VaultQuery, VaultStatus,
    DEFAULT_IDLE_TIMEOUT_SECONDS,
};

use super::NotesHandle;
use crate::AppState;

/// How often the watchdog thread checks for a clipboard that needs clearing.
const CLIPBOARD_TICK: Duration = Duration::from_millis(250);

/// Idle timer state for the interface; contains no secret.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct IdleStatus {
    pub timeout_seconds: u64,
    pub automatic: bool,
    /// Seconds left before the automatic lock, when one is armed.
    pub remaining_seconds: Option<u64>,
}

/// A freshly generated password, with its strength estimate.
///
/// The password is returned because the user asked to see it; it is not stored
/// until the user saves the item.
#[derive(Clone, Debug, Serialize)]
pub struct GeneratedPassword {
    pub password: String,
    pub entropy_bits: f64,
}

/// Result of generating a password directly onto the clipboard.
///
/// The password itself never crosses the bridge on this path.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct GeneratedSecretStatus {
    pub entropy_bits: f64,
    pub clipboard: ClipboardStatus,
}

/// Password vault state for the interface.
///
/// The vault store itself lives inside the shared session, so the derived vault
/// key never leaves the core: this handle only adds the idle timer and the
/// clipboard guard.
#[derive(Clone)]
pub struct VaultHandle {
    notes: NotesHandle,
    idle: Arc<Mutex<IdleLock>>,
    clipboard: Arc<Mutex<ClipboardGuard<SystemClipboard>>>,
    /// Runs after every lock, so in-memory text that belongs to the unlocked session is
    /// dropped with the keys. The spelling undo journal is the one such holder today.
    on_lock: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
}

impl Default for VaultHandle {
    fn default() -> Self {
        Self::new(NotesHandle::new())
    }
}

impl VaultHandle {
    /// Builds the handle and starts the clipboard watchdog.
    pub fn new(notes: NotesHandle) -> Self {
        let handle = Self {
            notes,
            idle: Arc::new(Mutex::new(IdleLock::new(DEFAULT_IDLE_TIMEOUT_SECONDS))),
            clipboard: Arc::new(Mutex::new(ClipboardGuard::new(SystemClipboard))),
            on_lock: Arc::new(Mutex::new(None)),
        };
        handle.spawn_clipboard_watchdog();
        handle
    }

    /// Registers an action to run after every lock.
    ///
    /// Every lock path goes through [`VaultHandle::lock_everything`], so one hook covers
    /// the idle timeout, an explicit lock, and application exit alike.
    pub fn set_lock_hook(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.on_lock.lock() = Some(hook);
    }

    fn spawn_clipboard_watchdog(&self) {
        let clipboard = Arc::clone(&self.clipboard);
        let _ = std::thread::Builder::new()
            .name("jarvis-vault-clipboard".to_string())
            .spawn(move || loop {
                std::thread::sleep(CLIPBOARD_TICK);
                let mut guard = clipboard.lock();
                // Clears only while the clipboard still holds our own value.
                let _ = guard.wipe_if_due(Instant::now());
            });
    }

    /// Locks everything when the idle timeout has elapsed. Returns whether it
    /// locked on this call.
    fn enforce_idle(&self) -> bool {
        let due = self.idle.lock().is_due(Instant::now());
        if due {
            self.lock_everything();
        }
        due
    }

    /// Records vault activity and refuses the call when the vault just locked
    /// itself because it was idle.
    fn touch_or_refuse(&self) -> Result<(), String> {
        if self.enforce_idle() {
            return Err(describe(VaultError::StorageLocked));
        }
        self.idle.lock().touch(Instant::now());
        Ok(())
    }

    /// Drops the vault key, the shared master key, and any pending clipboard
    /// secret.
    pub fn lock_everything(&self) {
        if let Ok(outcome) = self.clipboard.lock().cancel() {
            log::debug!("vault: clipboard on lock: {outcome:?}");
        }
        self.notes.lock();
        let hook = self.on_lock.lock().clone();
        if let Some(hook) = hook {
            // A hook that panicked must not undo the lock, which has already happened.
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| hook())).is_err() {
                log::warn!("vault: an on-lock action failed");
            }
        }
    }

    /// Locks on application exit.
    pub fn lock_for_exit(&self) {
        self.lock_everything();
    }

    fn with_vault<T>(
        &self,
        action: impl FnOnce(&mut EncryptedVaultStore) -> Result<T, VaultError>,
    ) -> Result<T, String> {
        self.touch_or_refuse()?;
        if !self.notes.is_unlocked() {
            return Err(describe(VaultError::StorageLocked));
        }
        self.notes
            .with_session(|session| session.with_store(action))
    }

    fn copy_secret(
        &self,
        secret: &str,
        clear_after_seconds: u64,
    ) -> Result<ClipboardStatus, String> {
        let mut guard = self.clipboard.lock();
        guard
            .copy_secret(secret, clear_after_seconds)
            .map_err(|error| {
                log::warn!("vault: {error}");
                error.to_string()
            })
    }

    fn clipboard_status(&self) -> ClipboardStatus {
        self.clipboard.lock().status()
    }
}

fn describe(error: VaultError) -> String {
    let message = error.to_string();
    // Locked is the expected state of the encrypted storage, not a fault: it is
    // locked at start-up, on the idle timer, and on every exit. A command that
    // arrives while it is locked asks for the password; it does not deserve a
    // warning that buries the real failures. Everything else stays a warning.
    if error == VaultError::StorageLocked {
        log::debug!("vault: storage is locked");
    } else {
        log::warn!("vault: {message}");
    }
    message
}

// ------------------------------------------------------------------- storage

#[tauri::command(async)]
pub fn vault_status(state: tauri::State<'_, AppState>) -> Result<VaultStatus, String> {
    // A status request is how the interface learns that the vault locked itself,
    // so it locks when due and then answers normally.
    state.vault.enforce_idle();
    state.notes.with_session(|session| session.status())
}

#[tauri::command(async)]
pub fn vault_initialize(
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<VaultStatus, String> {
    state.vault.touch_or_refuse()?;
    state
        .notes
        .with_session(|session| session.initialize(&password))
}

#[tauri::command(async)]
pub fn vault_unlock_password(
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<VaultStatus, String> {
    state.vault.touch_or_refuse()?;
    state
        .notes
        .with_session(|session| session.unlock_with_password(&password))
}

#[tauri::command(async)]
pub fn vault_unlock_dpapi(state: tauri::State<'_, AppState>) -> Result<VaultStatus, String> {
    state.vault.touch_or_refuse()?;
    state
        .notes
        .with_session(|session| session.unlock_with_dpapi())
}

#[tauri::command(async)]
pub fn vault_lock(state: tauri::State<'_, AppState>) -> Result<VaultStatus, String> {
    // The undo journal of the spelling feature holds document text, so it is dropped
    // together with the key it was produced under.
    state.autocorrect.clear_journals();
    state.vault.lock_everything();
    state.notes.with_session(|session| session.status())
}

#[tauri::command(async)]
pub fn vault_import_backup(
    state: tauri::State<'_, AppState>,
    envelope: String,
    password: String,
) -> Result<VaultStatus, String> {
    state.vault.touch_or_refuse()?;
    state
        .notes
        .with_session(|session| session.import_backup(&envelope, &password))
}

/// Reads a portable envelope from a chosen file and imports it.
///
/// Cancelling the picker is not an error.
#[tauri::command(async)]
pub fn vault_import_backup_file(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    password: String,
) -> Result<VaultStatus, String> {
    let Some(source) = open_backup_path(&app) else {
        return state.notes.with_session(|session| session.status());
    };
    let envelope = std::fs::read_to_string(&source).map_err(|_| describe(VaultError::VaultIo))?;
    state.vault.touch_or_refuse()?;
    state
        .notes
        .with_session(|session| session.import_backup(&envelope, &password))
}

/// Writes a fresh portable envelope to a chosen file. Returns the path, or an
/// empty string when the user cancels.
#[tauri::command(async)]
pub fn vault_export_backup_file(
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
    password: String,
) -> Result<String, String> {
    let Some(destination) = save_backup_path(&app) else {
        return Ok(String::new());
    };
    state.vault.touch_or_refuse()?;
    state.notes.with_session(|session| {
        session
            .export_backup_to(&password, &destination)
            .map(|path| path.display().to_string())
    })
}

/// Re-wraps the master key under a new master password.
#[tauri::command(async)]
pub fn vault_change_master_password(
    state: tauri::State<'_, AppState>,
    current: String,
    new_password: String,
) -> Result<MasterPasswordChange, String> {
    state.vault.touch_or_refuse()?;
    state
        .notes
        .with_session(|session| session.change_master_password(&current, &new_password))
}

// --------------------------------------------------------------- idle locking

#[tauri::command(async)]
pub fn vault_set_idle_timeout(
    state: tauri::State<'_, AppState>,
    seconds: u64,
) -> Result<IdleStatus, String> {
    let mut idle = state.vault.idle.lock();
    idle.set_timeout(seconds);
    let now = Instant::now();
    Ok(IdleStatus {
        timeout_seconds: idle.timeout_seconds(),
        automatic: idle.is_automatic(),
        remaining_seconds: idle.remaining_seconds(now),
    })
}

#[tauri::command(async)]
pub fn vault_idle_status(state: tauri::State<'_, AppState>) -> Result<IdleStatus, String> {
    state.vault.enforce_idle();
    let idle = state.vault.idle.lock();
    let now = Instant::now();
    Ok(IdleStatus {
        timeout_seconds: idle.timeout_seconds(),
        automatic: idle.is_automatic(),
        remaining_seconds: idle.remaining_seconds(now),
    })
}

/// Records interface activity, so moving the mouse or typing keeps the vault
/// unlocked without calling a data command.
#[tauri::command(async)]
pub fn vault_touch(state: tauri::State<'_, AppState>) -> Result<IdleStatus, String> {
    state.vault.touch_or_refuse()?;
    let idle = state.vault.idle.lock();
    let now = Instant::now();
    Ok(IdleStatus {
        timeout_seconds: idle.timeout_seconds(),
        automatic: idle.is_automatic(),
        remaining_seconds: idle.remaining_seconds(now),
    })
}

// ------------------------------------------------------------------- items

#[tauri::command(async)]
pub fn vault_list(
    state: tauri::State<'_, AppState>,
    query: VaultQuery,
) -> Result<VaultItemList, String> {
    state.vault.with_vault(|store| store.list_items(&query))
}

#[tauri::command(async)]
pub fn vault_get(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<Option<VaultItemDetails>, String> {
    state.vault.with_vault(|store| store.get_item(id))
}

#[tauri::command(async)]
pub fn vault_create(
    state: tauri::State<'_, AppState>,
    draft: VaultItemDraft,
) -> Result<VaultItemDetails, String> {
    state.vault.with_vault(|store| store.create_item(&draft))
}

#[tauri::command(async)]
pub fn vault_update(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    draft: VaultItemDraft,
) -> Result<VaultItemDetails, String> {
    state
        .vault
        .with_vault(|store| store.update_item(id, &draft))
}

/// Edits only the non-secret fields.
///
/// The interface uses this whenever the user has not revealed the secret, so a
/// rename or a retag can never erase a stored password.
#[tauri::command(async)]
pub fn vault_update_metadata(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    metadata: VaultMetadataDraft,
) -> Result<VaultItemDetails, String> {
    state
        .vault
        .with_vault(|store| store.update_metadata(id, &metadata))
}

/// Replaces the secret fields after an explicit reveal.
#[tauri::command(async)]
pub fn vault_update_secrets(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    password: String,
    notes: String,
) -> Result<VaultItemDetails, String> {
    state
        .vault
        .with_vault(|store| store.update_secrets(id, &password, &notes))
}

#[tauri::command(async)]
pub fn vault_set_favorite(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    favorite: bool,
) -> Result<VaultItemDetails, String> {
    state
        .vault
        .with_vault(|store| store.set_favorite(id, favorite))
}

#[tauri::command(async)]
pub fn vault_trash(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<VaultItemDetails, String> {
    state.vault.with_vault(|store| store.trash_item(id))
}

#[tauri::command(async)]
pub fn vault_restore(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<VaultItemDetails, String> {
    state.vault.with_vault(|store| store.restore_item(id))
}

#[tauri::command(async)]
pub fn vault_purge(state: tauri::State<'_, AppState>, id: Uuid) -> Result<(), String> {
    state.vault.with_vault(|store| store.purge_item(id))
}

#[tauri::command(async)]
pub fn vault_tags(state: tauri::State<'_, AppState>) -> Result<Vec<String>, String> {
    state.vault.with_vault(|store| store.all_tags())
}

// ----------------------------------------------------------------- secrets

/// Explicit reveal: the only command that returns a password to the interface.
#[tauri::command(async)]
pub fn vault_reveal(
    state: tauri::State<'_, AppState>,
    id: Uuid,
) -> Result<SecretRevealResult, String> {
    state.vault.with_vault(|store| {
        store.reveal_secret(id, jarvis_core::vault::DEFAULT_REVEAL_TIMEOUT_SECONDS)
    })
}

#[tauri::command(async)]
pub fn vault_copy_username(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    clear_after_seconds: u64,
) -> Result<ClipboardStatus, String> {
    let secret = Zeroizing::new(
        state
            .vault
            .with_vault(|store| store.username_for_clipboard(id))?,
    );
    state.vault.copy_secret(&secret, clear_after_seconds)
}

#[tauri::command(async)]
pub fn vault_copy_password(
    state: tauri::State<'_, AppState>,
    id: Uuid,
    clear_after_seconds: u64,
) -> Result<ClipboardStatus, String> {
    let secret = Zeroizing::new(
        state
            .vault
            .with_vault(|store| store.password_for_clipboard(id))?,
    );
    state.vault.copy_secret(&secret, clear_after_seconds)
}

#[tauri::command(async)]
pub fn vault_clipboard_status(
    state: tauri::State<'_, AppState>,
) -> Result<ClipboardStatus, String> {
    Ok(state.vault.clipboard_status())
}

#[tauri::command(async)]
pub fn vault_clipboard_clear(state: tauri::State<'_, AppState>) -> Result<ClipboardStatus, String> {
    let outcome = state.vault.clipboard.lock().clear_now().map_err(|error| {
        log::warn!("vault: {error}");
        error.to_string()
    })?;
    log::debug!("vault: clipboard cleared: {outcome:?}");
    Ok(state.vault.clipboard_status())
}

// --------------------------------------------------------------- generator

#[tauri::command(async)]
pub fn vault_generate_password(policy: PasswordPolicy) -> Result<GeneratedPassword, String> {
    let password = generate_password(&policy).map_err(describe)?;
    Ok(GeneratedPassword {
        password,
        entropy_bits: jarvis_core::vault::estimate_entropy_bits(&policy),
    })
}

/// Generates a password and puts it straight on the clipboard.
///
/// The generated secret never reaches the interface on this path.
#[tauri::command(async)]
pub fn vault_generate_and_copy(
    state: tauri::State<'_, AppState>,
    policy: PasswordPolicy,
    clear_after_seconds: u64,
) -> Result<GeneratedSecretStatus, String> {
    let password = Zeroizing::new(generate_password(&policy).map_err(describe)?);
    let clipboard = state.vault.copy_secret(&password, clear_after_seconds)?;
    Ok(GeneratedSecretStatus {
        entropy_bits: jarvis_core::vault::estimate_entropy_bits(&policy),
        clipboard,
    })
}

// --------------------------------------------------------------- conflicts

#[tauri::command(async)]
pub fn vault_conflicts(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<VaultConflictView>, String> {
    state.vault.with_vault(|store| store.conflicts())
}

#[tauri::command(async)]
pub fn vault_resolve_conflict(
    state: tauri::State<'_, AppState>,
    conflict: Uuid,
    resolution: VaultConflictResolution,
) -> Result<VaultConflictOutcome, String> {
    state
        .vault
        .with_vault(|store| store.resolve_conflict(conflict, resolution))
}

// ----------------------------------------------------------------- helpers

fn save_backup_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.dialog()
        .file()
        .set_title("JARVIS")
        .set_file_name("jarvis-key-backup.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
}

fn open_backup_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.dialog()
        .file()
        .set_title("JARVIS")
        .add_filter("JSON", &["json"])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
}
