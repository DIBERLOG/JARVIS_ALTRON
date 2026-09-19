//! Windows-action commands exposed to the interface.
//!
//! Rules that shape this file:
//!
//! * the interface never starts a program, never passes an executable path, never decides a
//!   risk level, and never mints a confirmation token. It asks for an action by name, receives
//!   a preview, and returns the token from that preview;
//! * a confirmation always executes the request the core stored, never anything the interface
//!   sends back, so a modified webview cannot change what was agreed to;
//! * the only path an executable can enter the allowlist through is the native file dialog in
//!   this file, on an explicit click;
//! * a pending action lives in the core only. Nothing about it is persisted in the webview,
//!   and a reload loses it on purpose;
//! * no command logs an action payload, a window title, a reminder text, or a path.
//!
//! There is no command here that runs a command line, ends a process, deletes a file, edits the
//! registry, shuts the machine down, reads the vault, or sends a screenshot anywhere.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;

use jarvis_core::notes::vault::VaultPaths;
use jarvis_core::windows_actions::{
    platform_backend, ActionError, ActionPreview, ActionRequestOutcome, ActionResult, ActionSource,
    AllowedApplication, AllowedApplicationDraft, AuditEntry, Capabilities, IdentityCheck,
    ScheduledView, ToolAvailability, VoiceRoute, WindowSummary, WindowsAction,
    WindowsActionSettings, WindowsActions,
};

use crate::AppState;

/// How many fired timers and reminders are held for the interface before the oldest is dropped.
///
/// The queue only carries what the window has not shown yet; the items themselves are on disk
/// and in the list command.
const MAX_FIRED_QUEUE: usize = 16;

/// The Windows-action session, shared by the interface, the voice host, and the model path.
pub struct WindowsActionsHandle {
    session: Arc<Mutex<WindowsActions>>,
    /// Fired timers and reminders the window has not been told about yet.
    fired: Arc<Mutex<VecDeque<ScheduledView>>>,
}

impl Clone for WindowsActionsHandle {
    fn clone(&self) -> Self {
        Self {
            session: Arc::clone(&self.session),
            fired: Arc::clone(&self.fired),
        }
    }
}

impl WindowsActionsHandle {
    /// Opens the feature over the application data directory with the platform backend.
    ///
    /// The session reads its own settings document, so there is one owner of "what the user
    /// chose" and no second copy in the application database that could disagree with it.
    pub fn restore() -> Self {
        let directory = data_directory();
        let settings = WindowsActionSettings::default_for(&directory);
        let session = WindowsActions::open(&directory, platform_backend(), settings);
        // The executor can only start what the allowlist holds; without this it can start
        // nothing, which is the safe default if the wiring below ever breaks.
        session.install_launch_lookup();
        Self {
            session: Arc::new(Mutex::new(session)),
            fired: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Opens the feature over an explicit directory, for tests.
    pub fn with_session(session: WindowsActions) -> Self {
        session.install_launch_lookup();
        Self {
            session: Arc::new(Mutex::new(session)),
            fired: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn session(&self) -> &Arc<Mutex<WindowsActions>> {
        &self.session
    }

    /// Installs the hook the scheduler calls when a timer or reminder fires.
    ///
    /// The hook stores the item and pokes the window; it never touches the platform, so a
    /// notification the interface shows can never be the only record of a fired reminder.
    pub fn install_fired_hook<F>(&self, poke: F)
    where
        F: Fn(&ScheduledView) + Send + Sync + 'static,
    {
        let queue = Arc::clone(&self.fired);
        let hook: jarvis_core::windows_actions::FiredHook = Arc::new(move |view: ScheduledView| {
            {
                let mut queue = queue.lock();
                while queue.len() >= MAX_FIRED_QUEUE {
                    queue.pop_front();
                }
                queue.push_back(view.clone());
            }
            poke(&view);
        });
        self.session.lock().set_fired_hook(hook);
    }

    /// The fired items the window has not seen, oldest first.
    pub fn take_fired(&self) -> Vec<ScheduledView> {
        let mut queue = self.fired.lock();
        queue.drain(..).collect()
    }

    /// Stops the scheduler thread. Called when the application exits.
    pub fn shutdown(&self) {
        self.session.lock().shutdown();
    }
}

impl std::fmt::Debug for WindowsActionsHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WindowsActionsHandle")
            .field("pending_fired", &self.fired.lock().len())
            .finish()
    }
}

/// Everything the settings page needs about the feature, in one call.
#[derive(Clone, Debug, Serialize)]
pub struct WindowsActionsOverview {
    pub capabilities: Capabilities,
    pub settings: WindowsActionSettings,
    pub tools: ToolAvailability,
    pub allowed_applications: Vec<AllowedApplicationView>,
    pub screenshots_directory: String,
    pub audit_entries: usize,
    /// Content-free rows of the central policy table, for the "what is allowed" list.
    pub policy: Vec<PolicyRowView>,
}

/// One row of the policy table, as the interface shows it.
#[derive(Clone, Debug, Serialize)]
pub struct PolicyRowView {
    pub action_type: String,
    pub risk: String,
    /// What the row means, in one short sentence.
    pub note: String,
    pub requires_confirmation: bool,
    pub forbidden: bool,
}

/// One allowed program, with the identity check the list command just performed.
#[derive(Clone, Debug, Serialize)]
pub struct AllowedApplicationView {
    pub id: String,
    pub display_name: String,
    pub executable_file_name: String,
    pub path: String,
    pub fixed_arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    /// Whether the file is still the one the user allowed.
    pub identity_changed: bool,
    /// Content-free reason when it changed.
    pub identity_reason: Option<String>,
}

impl AllowedApplicationView {
    fn of(application: &AllowedApplication, identity: IdentityCheck) -> Self {
        let (identity_changed, identity_reason) = match identity {
            IdentityCheck::Unchanged => (false, None),
            IdentityCheck::Changed { reason } => (true, Some(reason.to_string())),
        };
        Self {
            id: application.id.clone(),
            display_name: application.display_name.clone(),
            executable_file_name: application.executable_file_name(),
            path: application.canonical_executable_path.clone(),
            fixed_arguments: application.fixed_arguments.clone(),
            working_directory: application.working_directory.clone(),
            enabled: application.enabled,
            created_at: application.created_at.clone(),
            updated_at: application.updated_at.clone(),
            identity_changed,
            identity_reason,
        }
    }
}

// ---------------------------------------------------------------------------- commands

/// The capabilities, the settings, the policy table, and the allowed programs.
#[tauri::command]
pub async fn windows_actions_overview(
    state: tauri::State<'_, AppState>,
) -> Result<WindowsActionsOverview, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        let session = handle.session.lock();
        let allowed_applications = session
            .allowed_applications()
            .iter()
            .map(|application| {
                let identity = session
                    .allowed_application_identity(&application.id)
                    .unwrap_or(IdentityCheck::Changed {
                        reason: "unreadable",
                    });
                AllowedApplicationView::of(application, identity)
            })
            .collect();
        let policy = session
            .policy()
            .table()
            .iter()
            .map(|(action_type, risk, note)| PolicyRowView {
                action_type: action_type.to_string(),
                risk: risk.as_str().to_string(),
                requires_confirmation: *risk == jarvis_core::windows_actions::ActionRisk::Confirm,
                forbidden: *risk == jarvis_core::windows_actions::ActionRisk::Forbidden,
                note: note.to_string(),
            })
            .collect();
        // `false` says the client cannot carry structured tools yet; the model path reports
        // the honest reason instead of offering an action it cannot deliver.
        let (tools, _catalogue) = session.ai_tools(false);
        Ok(WindowsActionsOverview {
            capabilities: session.capabilities(),
            settings: session.settings().clone(),
            tools,
            screenshots_directory: session.executor().screenshot_settings().directory,
            audit_entries: session.audit_entries().len(),
            policy,
            allowed_applications,
        })
    })
    .await
}

#[tauri::command]
pub async fn windows_actions_update_settings(
    state: tauri::State<'_, AppState>,
    settings: WindowsActionSettings,
) -> Result<WindowsActionSettings, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        handle
            .session
            .lock()
            .update_settings(settings)
            .map_err(describe)
    })
    .await
}

/// Asks for one action. The answer is either the result or the preview that must be confirmed.
#[tauri::command]
pub async fn windows_actions_request(
    state: tauri::State<'_, AppState>,
    action: WindowsAction,
    source: ActionSource,
) -> Result<ActionRequestOutcome, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        handle
            .session
            .lock()
            .request(action, source)
            .map_err(describe)
    })
    .await
}

/// The action waiting for the user, if any. The token in the preview is the only one that works.
#[tauri::command]
pub async fn windows_actions_pending(
    state: tauri::State<'_, AppState>,
) -> Result<Option<ActionPreview>, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || Ok(handle.session.lock().pending_preview())).await
}

/// Confirms the pending action and runs exactly what was stored.
#[tauri::command]
pub async fn windows_actions_confirm(
    state: tauri::State<'_, AppState>,
    token: String,
) -> Result<ActionResult, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        handle
            .session
            .lock()
            .confirm(&token)
            .map_err(describe)
    })
    .await
}

/// Refuses the pending action.
#[tauri::command]
pub async fn windows_actions_cancel(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || Ok(handle.session.lock().cancel())).await
}

/// The visible windows of the moment. The identifiers are short-lived by design.
#[tauri::command]
pub async fn windows_actions_list_windows(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<WindowSummary>, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || handle.session.lock().list_windows().map_err(describe)).await
}

/// The timers and reminders, oldest deadline first for the active ones.
#[tauri::command]
pub async fn windows_actions_scheduled(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ScheduledView>, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || Ok(handle.session.lock().scheduled())).await
}

/// The timers and reminders that fired while the window was not looking.
#[tauri::command]
pub async fn windows_actions_take_fired(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ScheduledView>, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || Ok(handle.take_fired())).await
}

/// Removes fired and cancelled items from the list.
#[tauri::command]
pub async fn windows_actions_prune_scheduled(
    state: tauri::State<'_, AppState>,
) -> Result<usize, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || handle.session.lock().prune_scheduled().map_err(describe)).await
}

/// The audit log, newest first. It holds no titles, no paths, and no text.
#[tauri::command]
pub async fn windows_actions_audit_log(
    state: tauri::State<'_, AppState>,
    limit: usize,
) -> Result<Vec<AuditEntry>, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || Ok(handle.session.lock().recent_audit(limit.min(500)))).await
}

/// Clears the audit log. Requires an explicit confirmation from the interface.
#[tauri::command]
pub async fn windows_actions_clear_audit_log(
    state: tauri::State<'_, AppState>,
    confirmed: bool,
) -> Result<(), String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        handle
            .session
            .lock()
            .clear_audit(confirmed)
            .map_err(describe)
    })
    .await
}

/// Writes the audit log to a file the user picks. Requires an explicit confirmation.
#[tauri::command]
pub async fn windows_actions_export_audit_log(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    confirmed: bool,
) -> Result<Option<usize>, String> {
    if !confirmed {
        return Err(describe(ActionError::InvalidArguments {
            detail: "the export needs a confirmation".to_string(),
        }));
    }
    let Some(destination) = save_audit_path(&app) else {
        return Ok(None);
    };
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        handle
            .session
            .lock()
            .export_audit(&destination, true)
            .map(Some)
            .map_err(describe)
    })
    .await
}

/// Adds a program to the allowlist.
///
/// The executable is chosen in the native file dialog here, so the interface never sends a
/// path, and the core validates and fingerprints the file the user actually picked.
#[tauri::command]
pub async fn windows_actions_add_allowed_application(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    display_name: String,
    fixed_arguments: Option<Vec<String>>,
    working_directory: Option<String>,
) -> Result<Option<AllowedApplicationView>, String> {
    let Some(program) = pick_executable(&app) else {
        return Ok(None);
    };
    let draft = AllowedApplicationDraft {
        display_name,
        path: program.to_string_lossy().into_owned(),
        fixed_arguments: fixed_arguments.unwrap_or_default(),
        working_directory,
    };
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        let mut session = handle.session.lock();
        let added = session.add_allowed_application(&draft).map_err(describe)?;
        let identity = session
            .allowed_application_identity(&added.id)
            .unwrap_or(IdentityCheck::Unchanged);
        Ok(Some(AllowedApplicationView::of(&added, identity)))
    })
    .await
}

/// Removes a program from the allowlist.
#[tauri::command]
pub async fn windows_actions_remove_allowed_application(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<AllowedApplicationView, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        let mut session = handle.session.lock();
        let removed = session.remove_allowed_application(&id).map_err(describe)?;
        Ok(AllowedApplicationView::of(
            &removed,
            IdentityCheck::Unchanged,
        ))
    })
    .await
}

/// Switches an entry on or off without forgetting it.
#[tauri::command]
pub async fn windows_actions_set_allowed_application_enabled(
    state: tauri::State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<AllowedApplicationView, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        let mut session = handle.session.lock();
        let updated = session
            .set_allowed_application_enabled(&id, enabled)
            .map_err(describe)?;
        let identity = session
            .allowed_application_identity(&updated.id)
            .unwrap_or(IdentityCheck::Unchanged);
        Ok(AllowedApplicationView::of(&updated, identity))
    })
    .await
}

/// Accepts the current bytes of an executable that changed since it was allowed.
#[tauri::command]
pub async fn windows_actions_reaccept_allowed_application(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<AllowedApplicationView, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        let mut session = handle.session.lock();
        let updated = session
            .reaccept_allowed_application(&id)
            .map_err(describe)?;
        let identity = session
            .allowed_application_identity(&updated.id)
            .unwrap_or(IdentityCheck::Unchanged);
        Ok(AllowedApplicationView::of(&updated, identity))
    })
    .await
}

/// Routes a spoken phrase through the same pipeline the buttons use.
#[tauri::command]
pub async fn windows_actions_route_voice(
    state: tauri::State<'_, AppState>,
    text: String,
) -> Result<VoiceRoute, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        handle
            .session
            .lock()
            .route_voice(&text)
            .map_err(describe)
    })
    .await
}

/// The structured tools the local model may call, as the interface documents them.
#[tauri::command]
pub async fn windows_actions_tools(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<jarvis_core::windows_actions::ToolDefinition>, String> {
    let handle = state.windows_actions.clone();
    on_blocking(move || {
        let session = handle.session.lock();
        let (_availability, catalogue) = session.ai_tools(false);
        Ok(catalogue)
    })
    .await
}

// ----------------------------------------------------------------------------- helpers

/// Runs a blocking job on the blocking pool: no action touches the disk on the UI thread.
async fn on_blocking<T, F>(action: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(action).await {
        Ok(result) => result,
        Err(_) => Err(describe(ActionError::StorageError)),
    }
}

/// The application data directory, from the same paths the encrypted storage uses.
///
/// The feature keeps its own directory there, next to `timers.json`, the allowlist, and the
/// audit log.
fn data_directory() -> PathBuf {
    VaultPaths::production()
        .map(|paths| paths.data_dir)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Turns an action error into a message that is safe to show and to log.
///
/// The error codes are content-free by construction; nothing here formats a payload.
fn describe(error: ActionError) -> String {
    let message = error.to_string();
    log::warn!("windows actions: {}", error.code());
    message
}

fn pick_executable(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS")
        .add_filter("Program", &["exe"])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
}

fn save_audit_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS")
        .set_file_name("jarvis-actions-audit.jsonl")
        .add_filter("JSON Lines", &["jsonl"])
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
}
