//! Local AI commands exposed to the interface.
//!
//! Rules that shape this file:
//!
//! * the interface never spawns a process and never speaks HTTP: it asks the
//!   gateway for a status, for a validation report, and for a generation;
//! * starting the server waits for it to answer, so it runs on a blocking worker
//!   thread and never on the window thread;
//! * tokens arrive through a Tauri channel, which the interface renders as they
//!   appear instead of polling once per token;
//! * no prompt, completion, or stderr line is logged here: the core already keeps
//!   messages content-free, and this module only forwards them;
//! * this module holds no handle to the encrypted storages. Their commands and
//!   handles live in their own modules; the AI runtime is given a configuration
//!   and nothing else, which is what makes the separation structural rather than
//!   a promise. A test scans this file and fails if that ever changes.

use std::sync::Arc;

use tauri::ipc::Channel;
use tauri_plugin_dialog::DialogExt;

use jarvis_core::ai::local::{
    GenerationEvent, GenerationRequest, LocalAiConfig, LocalAiGateway, LocalAiReport,
    LocalAiStatus, SETTINGS_KEY,
};
use jarvis_core::ai::ChatError;
use jarvis_core::SettingsManager;

use crate::AppState;

/// The single local AI runtime of the application.
///
/// Cloning the handle shares one gateway, and therefore one managed server
/// process: two windows cannot each start their own `llama-server`.
#[derive(Clone)]
pub struct LocalAiHandle {
    gateway: Arc<LocalAiGateway>,
}

impl Default for LocalAiHandle {
    fn default() -> Self {
        Self::new(LocalAiConfig::default())
    }
}

impl LocalAiHandle {
    pub fn new(config: LocalAiConfig) -> Self {
        Self {
            gateway: Arc::new(LocalAiGateway::new(config)),
        }
    }

    /// Builds the gateway from the stored settings.
    pub fn restore(settings: &SettingsManager) -> Self {
        Self::new(stored_config(settings))
    }

    pub fn gateway(&self) -> &LocalAiGateway {
        &self.gateway
    }

    /// Whether the running model can carry structured tool calls.
    ///
    /// This is the probe's answer, not a guess: a model whose template cannot express a call is
    /// never offered the action catalogue, because the alternative would be reading an action
    /// out of its prose.
    pub fn tools_supported(&self) -> bool {
        self.gateway
            .status()
            .map(|status| status.capabilities.tools_in_template)
            .unwrap_or(false)
    }

    /// The shared gateway, for a caller that needs to keep it alive on a worker.
    ///
    /// The memory summarizer uses this so it runs against the same managed server
    /// instead of creating a second client.
    pub fn shared(&self) -> Arc<LocalAiGateway> {
        Arc::clone(&self.gateway)
    }

    /// The configuration the gateway is currently using.
    pub fn config(&self) -> LocalAiConfig {
        self.gateway.config()
    }

    /// Stops the managed server. Called when the application exits so the child
    /// process never outlives the window.
    pub fn shutdown(&self) {
        self.gateway.shutdown();
    }
}

/// Reads the stored local AI settings.
///
/// A missing, empty, or damaged value means defaults: a bad settings string must
/// never stop the application from starting, and the interface can always show a
/// usable form.
pub fn stored_config(settings: &SettingsManager) -> LocalAiConfig {
    match settings.read(SETTINGS_KEY) {
        Some(stored) if !stored.trim().is_empty() => {
            LocalAiConfig::from_json(&stored).unwrap_or_default()
        }
        _ => LocalAiConfig::default(),
    }
}

/// Turns a core error into a message that is safe to show and to log.
fn describe(error: ChatError) -> String {
    let message = error.to_string();
    log::warn!("local ai: {message}");
    message
}

/// Runs a blocking gateway call on the blocking pool.
///
/// Readiness probing can wait for the whole startup timeout, so it must not hold
/// an async worker, and it must certainly not hold the window thread.
async fn on_blocking<T, F>(action: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(action).await {
        Ok(result) => result,
        Err(_) => {
            let message = ChatError::ProcessUnavailable.to_string();
            log::warn!("local ai: {message}");
            Err(message)
        }
    }
}

// ------------------------------------------------------------------ settings

/// The configuration the runtime is currently using.
#[tauri::command(async)]
pub fn local_ai_get_config(state: tauri::State<'_, AppState>) -> Result<LocalAiConfig, String> {
    Ok(state.local_ai.config())
}

/// Checks a configuration without saving or starting anything.
///
/// This is what the interface calls while the user edits the form, so problems
/// (missing files, a refused host, too little memory) are visible before the
/// server is ever spawned.
#[tauri::command(async)]
pub fn local_ai_validate(config: LocalAiConfig) -> Result<LocalAiReport, String> {
    // Validation starts no process, so it needs no managed gateway.
    Ok(LocalAiGateway::default().validate(&config))
}

/// Stores the configuration and applies it to the runtime.
///
/// The configuration is saved even when it is incomplete: the user is still
/// filling the form. Starting the server is what refuses a blocked
/// configuration, and the returned report says exactly why.
#[tauri::command(async)]
pub fn local_ai_save_config(
    state: tauri::State<'_, AppState>,
    config: LocalAiConfig,
) -> Result<LocalAiReport, String> {
    let encoded = config.to_json().map_err(describe)?;
    state
        .settings
        .write(SETTINGS_KEY, &encoded)
        .map_err(|error| {
            log::warn!("local ai: settings could not be saved: {error}");
            error
        })?;
    state
        .local_ai
        .gateway()
        .apply_config(config.clone())
        .map_err(describe)?;
    Ok(state.local_ai.gateway().validate(&config))
}

/// Writes the AI settings to a file the user chooses.
///
/// Returns an empty string when the picker is cancelled. The file holds paths and
/// sampling values only; nothing secret is ever written.
#[tauri::command(async)]
pub fn local_ai_export_config(
    app: tauri::AppHandle,
    config: LocalAiConfig,
) -> Result<String, String> {
    let Some(destination) = save_config_path(&app) else {
        return Ok(String::new());
    };
    config
        .save_to_file(&destination)
        .map_err(describe)
        .map(|()| destination.display().to_string())
}

/// Reads AI settings from a file the user chooses.
///
/// Returns `None` when the picker is cancelled. The caller decides whether to
/// save the imported values.
#[tauri::command(async)]
pub fn local_ai_import_config(app: tauri::AppHandle) -> Result<Option<LocalAiConfig>, String> {
    let Some(source) = open_config_path(&app) else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&source).map_err(|_| {
        describe(ChatError::InvalidConfiguration(
            "the settings file could not be read".to_string(),
        ))
    })?;
    LocalAiConfig::from_json(&text).map(Some).map_err(describe)
}

// ----------------------------------------------------------------- lifecycle

/// Starts the managed server and waits until it answers.
///
/// Idempotent: calling it while the server is already running returns the current
/// status instead of starting a second process. The application starts exactly
/// one `llama-server`, and only this process is ever stopped.
#[tauri::command]
pub async fn local_ai_start(state: tauri::State<'_, AppState>) -> Result<LocalAiStatus, String> {
    let handle = state.local_ai.clone();
    on_blocking(move || handle.gateway().start().map_err(describe)).await
}

/// Stops the managed server that this application started.
#[tauri::command]
pub async fn local_ai_stop(state: tauri::State<'_, AppState>) -> Result<LocalAiStatus, String> {
    let handle = state.local_ai.clone();
    on_blocking(move || handle.gateway().stop().map_err(describe)).await
}

/// Restarts the managed server, for example after changing the model.
#[tauri::command]
pub async fn local_ai_restart(state: tauri::State<'_, AppState>) -> Result<LocalAiStatus, String> {
    let handle = state.local_ai.clone();
    on_blocking(move || handle.gateway().restart().map_err(describe)).await
}

/// Current state, capabilities, and the bounded stderr tail.
#[tauri::command(async)]
pub fn local_ai_status(state: tauri::State<'_, AppState>) -> Result<LocalAiStatus, String> {
    state.local_ai.gateway().status().map_err(describe)
}

// ---------------------------------------------------------------- generation

/// Starts a generation and streams its events through `channel`.
///
/// Returns the generation id. The work happens on a worker thread in the core, so
/// this command returns immediately and the window stays responsive; cancel with
/// [`local_ai_cancel`].
#[tauri::command(async)]
pub fn local_ai_generate(
    state: tauri::State<'_, AppState>,
    channel: Channel<GenerationEvent>,
    request: GenerationRequest,
) -> Result<String, String> {
    let sink: jarvis_core::ai::local::EventSink = Arc::new(move |event: GenerationEvent| {
        // Only the event kind is logged, never its text.
        if let Err(error) = channel.send(event) {
            log::debug!("local ai: the event channel is closed: {error}");
        }
    });
    state
        .local_ai
        .gateway()
        .start_generation(request, sink)
        .map(|handle| handle.id().to_string())
        .map_err(describe)
}

/// Requests cancellation of the running generation. Returns whether one was
/// running. Idempotent.
#[tauri::command(async)]
pub fn local_ai_cancel(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.local_ai.gateway().cancel())
}

// ------------------------------------------------------------------- pickers

/// Picks the `llama-server` executable. Returns `None` when cancelled.
#[tauri::command(async)]
pub fn local_ai_select_server(app: tauri::AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("llama-server")
        .add_filter("llama-server", &["exe"])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.display().to_string())
}

/// Picks the GGUF model file. Returns `None` when cancelled.
#[tauri::command(async)]
pub fn local_ai_select_model(app: tauri::AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("GGUF")
        .add_filter("GGUF model", &["gguf"])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
        .map(|path| path.display().to_string())
}

fn save_config_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.dialog()
        .file()
        .set_title("JARVIS")
        .set_file_name("jarvis-local-ai.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file()
        .and_then(|path| path.into_path().ok())
}

fn open_config_path(app: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    app.dialog()
        .file()
        .set_title("JARVIS")
        .add_filter("JSON", &["json"])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
}
