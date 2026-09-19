//! Backup and restore, exposed to the window.
//!
//! Rules that shape this module:
//!
//! * the window never chooses a path by typing one: the native dialog does, and
//!   the core only ever sees the file the person picked;
//! * nothing is overwritten and nothing is restored without an explicit
//!   confirmation flag, and the confirmation is the second call, not a default;
//! * one operation at a time. A second export or restore while one is running is
//!   refused with a code instead of racing the first;
//! * the plaintext of a restore lives in a staging directory inside the
//!   application data directory and is removed on every path, including a
//!   failure;
//! * the password is never logged, never stored, and never part of a report.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;

use jarvis_core::backup::{
    self, BackupError, BackupPreview, BackupRoots, BackupStatus, ExportPlan, ExportReport, Limits,
    RestorePlan, RestoreReport, COMPONENTS, LIMITS,
};
use jarvis_core::notes::vault::{VaultPaths, DPAPI_KEY_FILE};

use crate::tauri_commands::NotesHandle;
use crate::AppState;

/// An extension the container is offered with in the save dialog.
pub const CONTAINER_EXTENSION: &str = "jarvisbak";

/// What the panel shows before an export.
#[derive(Clone, Debug, Serialize)]
pub struct BackupPlanView {
    /// Logical component names, and whether they exist on this machine.
    pub included: Vec<String>,
    pub absent: Vec<String>,
    /// Whether the feature can run at all: a key envelope has to exist.
    pub available: bool,
    /// Whether a portable key envelope is present.
    pub key_envelope_present: bool,
}

/// The state of the feature, with the last operation of this process.
#[derive(Clone, Debug, Serialize)]
pub struct BackupPanelView {
    pub status: BackupStatus,
    pub plan: BackupPlanView,
}

/// The backup feature, and the one operation it may be running.
#[derive(Clone)]
pub struct BackupHandle {
    roots: BackupRoots,
    /// The encrypted storage, so a restore can close it and open it again.
    notes: NotesHandle,
    /// Whether an operation is running. Held for the whole operation.
    running: Arc<AtomicBool>,
    /// Cancels a restore that is in flight, for a full exit.
    cancel: Arc<AtomicBool>,
    /// The last operation and its outcome, for the panel and the diagnostics.
    last: Arc<Mutex<LastOperation>>,
}

#[derive(Clone, Debug, Default)]
struct LastOperation {
    name: Option<String>,
    code: Option<String>,
}

impl BackupHandle {
    /// Opens the feature over the production paths and recovers an interrupted
    /// restore before anything else is opened.
    pub fn restore(notes: NotesHandle) -> Self {
        let roots = match BackupRoots::production() {
            Ok(roots) => roots,
            Err(_) => BackupRoots {
                data_dir: PathBuf::from("."),
                config_dir: PathBuf::from("."),
            },
        };
        // An interrupted restore is finished or undone here, before a store is
        // opened: the alternative is a mixture of old and new databases that
        // nothing can reason about.
        match backup::restore::recover_interrupted(&roots) {
            Ok(Some(stage)) => log::warn!(
                "backup: an interrupted restore was rolled back (stage={})",
                backup::stage_name(stage)
            ),
            Ok(None) => {}
            Err(error) => log::error!(
                "backup: an interrupted restore could not be recovered (error_code={})",
                error.code()
            ),
        }
        Self {
            roots,
            notes,
            running: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            last: Arc::new(Mutex::new(LastOperation::default())),
        }
    }

    /// The state of the feature, for the panel and the diagnostics report.
    pub fn status(&self) -> BackupStatus {
        let last = self.last.lock().clone();
        BackupStatus::read(&self.roots).with_last(last.name.as_deref(), last.code.as_deref())
    }

    fn plan(&self) -> BackupPlanView {
        let report = backup::describe_current(&self.roots);
        BackupPlanView {
            included: report.captured,
            absent: report.absent,
            available: self
                .roots
                .data_dir
                .join(backup::KEY_ENVELOPE_FILE)
                .is_file(),
            key_envelope_present: self
                .roots
                .data_dir
                .join(backup::KEY_ENVELOPE_FILE)
                .is_file(),
        }
    }

    /// Runs one operation, refusing a second.
    fn run<T>(
        &self,
        name: &str,
        action: impl FnOnce() -> Result<T, BackupError>,
    ) -> Result<T, String> {
        if self
            .running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(BackupError::Busy.code().to_string());
        }
        self.cancel.store(false, Ordering::SeqCst);
        let outcome = action();
        self.running.store(false, Ordering::SeqCst);
        let code = outcome.as_ref().err().map(|error| error.code().to_string());
        match &code {
            Some(code) => log::warn!("backup: {name} failed (error_code={code})"),
            None => log::info!("backup: {name} finished"),
        }
        *self.last.lock() = LastOperation {
            name: Some(name.to_string()),
            code,
        };
        outcome.map_err(|error| error.code().to_string())
    }

    /// Whether an operation is running, for the exit step.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Stops what a restore is doing, for a full exit.
    ///
    /// A restore that is between the move and the install must not be cut in
    /// half: the flag is read at the safe points, and the journal is what makes
    /// the interrupted result recoverable on the next start either way.
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

/// The state of the feature, and what a full backup would contain.
#[tauri::command]
pub async fn backup_status(state: tauri::State<'_, AppState>) -> Result<BackupPanelView, String> {
    let handle = state.backup.clone();
    Ok(BackupPanelView {
        status: handle.status(),
        plan: handle.plan(),
    })
}

/// Creates a full backup. The destination is chosen in the native dialog.
///
/// `overwrite` is false unless the window asked the person and they said yes; an
/// existing file is refused otherwise, and the core never decides that on its
/// own.
#[tauri::command]
pub async fn backup_export(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    password: String,
    overwrite: bool,
) -> Result<ExportReport, String> {
    let handle = state.backup.clone();
    let suggested = format!(
        "jarvis-backup-{}.{CONTAINER_EXTENSION}",
        backup::now_compact()
    );
    let Some(destination) = save_dialog(&app, &suggested)? else {
        return Err("cancelled".to_string());
    };
    handle.run("export", || {
        let plan = ExportPlan {
            roots: handle.roots.clone(),
            components: COMPONENTS.to_vec(),
            overwrite,
            limits: LIMITS,
        };
        backup::export(&plan, &destination, password.as_bytes())
    })
}

/// Opens a container and reports what it holds, without changing anything.
#[tauri::command]
pub async fn backup_inspect(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    password: String,
) -> Result<Option<BackupPreview>, String> {
    let handle = state.backup.clone();
    let Some(source) = pick_dialog(&app)? else {
        return Ok(None);
    };
    let cancel = Arc::clone(&handle.cancel);
    let preview = handle.run("inspect", || {
        backup::inspect(&source, password.as_bytes(), &cancel, &LIMITS)
    })?;
    Ok(Some(preview))
}

/// Everything the window needs about the paths a restore touches.
#[derive(Clone, Debug, Serialize)]
pub struct RestoreOutcome {
    pub report: RestoreReport,
    /// Whether the encrypted storage was closed and re-opened, and is locked.
    pub storage_locked: bool,
    /// File name of the safety backup, so the person can find and delete it.
    pub safety_backup: Option<String>,
    /// Whether the replaced state is still on disk.
    pub previous_state: bool,
}

/// Restores a container over the current state.
///
/// The sequence is the engine's: verify, stage, safety backup, move aside,
/// install, verify, re-bind the local key, commit — with a rollback on any
/// failure. This command adds the part only the application can do: closing the
/// encrypted storage before the files are replaced, and opening it again
/// afterwards, locked.
#[tauri::command]
pub async fn backup_restore(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    password: String,
    confirmed: bool,
) -> Result<Option<RestoreOutcome>, String> {
    if !confirmed {
        // A restore replaces everything. It happens because a person said so.
        return Err("confirmation_required".to_string());
    }
    let handle = state.backup.clone();
    let notes = handle.notes.clone();
    let Some(source) = pick_dialog(&app)? else {
        return Ok(None);
    };

    // Nothing may be recording, generating, or writing while the databases are
    // replaced: the storage is closed first, and that closes every connection.
    notes.lock();

    let roots = handle.roots.clone();
    let cancel = Arc::clone(&handle.cancel);
    let bind_path = VaultPaths::production()
        .map(|paths| paths.data_dir.join(DPAPI_KEY_FILE))
        .unwrap_or_else(|_| roots.data_dir.join(DPAPI_KEY_FILE));
    let outcome = handle.run("restore", move || {
        let plan = RestorePlan {
            roots: roots.clone(),
            container: source,
            components: COMPONENTS.to_vec(),
            safety_backup: true,
            bind: Box::new(backup::restore::DpapiKeyBinding { path: bind_path }),
            limits: Limits::default(),
        };
        backup::restore::restore(&plan, password.as_bytes(), &cancel)
    });

    // Whether the restore succeeded or failed, the storage is opened again — and
    // it is opened locked, so the vault and the notes ask for the password before
    // anything is decrypted.
    notes.reset();
    notes.preload();

    let report = outcome?;
    let previous_state = handle.roots.data_dir.join(backup::PREVIOUS_DIR).is_dir();
    log::info!(
        "backup: restore finished (components={} safety={} rolled_back={:?})",
        report.restored.len(),
        report.safety_backup.is_some(),
        report.rolled_back
    );
    Ok(Some(RestoreOutcome {
        safety_backup: report.safety_backup.clone(),
        report,
        storage_locked: !notes.is_unlocked(),
        previous_state,
    }))
}

/// Deletes the state a previous restore replaced, when the person asks.
///
/// It is never deleted automatically: it is the only thing that can undo a
/// restore the person decides they did not want.
#[tauri::command]
pub async fn backup_discard_previous(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let handle = state.backup.clone();
    let previous = handle.roots.data_dir.join(backup::PREVIOUS_DIR);
    if !previous.is_dir() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&previous).map_err(|_| "storage".to_string())?;
    log::info!("backup: the previous state was deleted by the user");
    Ok(true)
}

/// Deletes one safety backup, by file name.
///
/// The name is checked against the same logical-name rules the container uses,
/// so a name from the window can never reach outside the safety directory.
#[tauri::command]
pub async fn backup_delete_safety(
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<bool, String> {
    let handle = state.backup.clone();
    let safe = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && name.ends_with(&format!(".{CONTAINER_EXTENSION}"))
        && !name.contains("..");
    if !safe {
        return Err("unsafe_entry_name".to_string());
    }
    let path = handle.roots.data_dir.join(backup::SAFETY_DIR).join(&name);
    if !path.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(&path).map_err(|_| "storage".to_string())?;
    Ok(true)
}

/// The native "save as" dialog, with the container extension.
fn save_dialog(app: &tauri::AppHandle, suggested: &str) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    Ok(app
        .dialog()
        .file()
        .set_title("JARVIS")
        .set_file_name(suggested)
        .add_filter("JARVIS backup", &[CONTAINER_EXTENSION])
        .blocking_save_file()
        .and_then(|path| path.into_path().ok()))
}

/// The native "open" dialog, for a container.
fn pick_dialog(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    use tauri_plugin_dialog::DialogExt;
    Ok(app
        .dialog()
        .file()
        .set_title("JARVIS")
        .add_filter("JARVIS backup", &[CONTAINER_EXTENSION])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok()))
}
