//! The Tauri side of the desktop shell: the tray icon, the single-instance
//! hand-off, the close button, autostart, and the commands the window calls.
//!
//! Every decision lives in `jarvis_core::desktop`: this file renders the menu
//! that module builds, dispatches on the identifiers it returns, and keeps one
//! `AudioSession` as the single answer to "is the microphone open".
//!
//! What this file does not do: it does not decide what a close means, it does
//! not write two settings documents, it does not create a second lifecycle, and
//! it does not put a secret into a label.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};

use jarvis_core::desktop::{
    tray_menu, tray_tooltip, AiSnapshot, AudioSession, AutostartBackend, AutostartController,
    AutostartState, AutostartStatus, CloseBehavior, DesktopSettings, DesktopSnapshot, DesktopStore,
    MicrophoneState, SetupState,
};
use jarvis_core::whisper::WhisperSettings;

use crate::AppState;

/// Identifier of the tray icon. There is one, so a second is a bug.
pub const TRAY_ID: &str = "jarvis-main";
/// Event the window receives when the close button was pressed and the answer is
/// "ask". The window then shows its own dialog: a native message box cannot
/// offer "remember this choice".
pub const CLOSE_REQUESTED_EVENT: &str = "desktop-close-requested";
/// Event the window receives when the tray menu changed the state underneath it.
pub const STATE_CHANGED_EVENT: &str = "desktop-state-changed";

/// The shell, shared by the tray, the close handler, and the commands.
pub struct DesktopHandle {
    settings: Mutex<DesktopSettings>,
    setup: Mutex<SetupState>,
    audio: Arc<AudioSession>,
    store: DesktopStore,
    autostart: AutostartController,
}

impl DesktopHandle {
    /// Opens the shell over the application data directory.
    pub fn new(app: &AppHandle) -> Self {
        let directory = data_directory();
        let store = DesktopStore::new(&directory);
        let settings = store.settings();
        let setup = store.setup();
        let autostart = AutostartController::new(
            Box::new(PluginAutostart { app: app.clone() }),
            current_executable(),
        );
        let handle = Self {
            settings: Mutex::new(settings),
            setup: Mutex::new(setup),
            audio: Arc::new(AudioSession::new()),
            store,
            autostart,
        };
        // An autostart entry that points at an older location is rewritten on
        // every start where the setting says it should exist: a move or an
        // update must not leave a Run key pointing at nothing.
        if handle.settings.lock().autostart_enabled {
            if let Err(error) = handle.autostart.enable() {
                log::warn!("desktop: autostart could not be restored: {}", error.code());
            }
        }
        handle
    }

    pub fn settings(&self) -> DesktopSettings {
        self.settings.lock().clone()
    }

    /// Stores the settings, atomically, and applies what can be applied now.
    pub fn update_settings(&self, settings: DesktopSettings) -> Result<DesktopSettings, String> {
        let previous = self.settings();
        let settings = settings.normalized();
        self.store
            .save_settings(&settings)
            .map_err(|error| error.to_string())?;
        // Autostart is the one setting with a side effect on the machine.
        if settings.autostart_enabled != previous.autostart_enabled {
            let result = if settings.autostart_enabled {
                self.autostart.enable().map(|_| ())
            } else {
                self.autostart.disable().map(|_| ())
            };
            if let Err(error) = result {
                // The setting is kept as the user asked; the state reports the
                // refusal, so the page can explain it instead of lying.
                log::warn!("desktop: autostart refused: {}", error.code());
                *self.settings.lock() = settings.clone();
                return Ok(settings);
            }
        }
        *self.settings.lock() = settings.clone();
        Ok(settings)
    }

    pub fn setup(&self) -> SetupState {
        self.setup.lock().clone()
    }

    pub fn save_setup(&self, state: &SetupState) -> Result<(), String> {
        self.store
            .save_setup(state)
            .map_err(|error| error.to_string())
    }

    pub fn audio(&self) -> &Arc<AudioSession> {
        &self.audio
    }

    pub fn autostart_status(&self) -> AutostartStatus {
        self.autostart.status()
    }

    /// Autostart on, as the platform reports it.
    pub fn autostart_enable(&self) -> Result<AutostartStatus, String> {
        let status = self.autostart.enable().map_err(|error| error.to_string())?;
        {
            let mut settings = self.settings.lock();
            settings.autostart_enabled = true;
            let copy = settings.clone();
            drop(settings);
            let _ = self.store.save_settings(&copy);
        }
        Ok(status)
    }

    pub fn autostart_disable(&self) -> Result<AutostartStatus, String> {
        let status = self
            .autostart
            .disable()
            .map_err(|error| error.to_string())?;
        {
            let mut settings = self.settings.lock();
            settings.autostart_enabled = false;
            let copy = settings.clone();
            drop(settings);
            let _ = self.store.save_settings(&copy);
        }
        Ok(status)
    }

    /// What the tray and the window both show.
    pub fn snapshot(&self, app: &AppHandle) -> DesktopSnapshot {
        let state = app.state::<AppState>();
        let window_visible = app
            .get_webview_window("main")
            .and_then(|window| window.is_visible().ok())
            .unwrap_or(false);
        // The gateway's own state is the only source for this: a stopped server
        // with a model configured is "stopped", and no model at all is "not
        // configured".
        let configured = state.local_ai.config().model_path().is_some();
        let ai_state = match state.local_ai.gateway().state() {
            jarvis_core::ai::local::LocalAiState::Ready => AiSnapshot::Ready,
            jarvis_core::ai::local::LocalAiState::Generating => AiSnapshot::Busy,
            jarvis_core::ai::local::LocalAiState::Starting
            | jarvis_core::ai::local::LocalAiState::Stopping => AiSnapshot::Busy,
            _ => {
                if configured {
                    AiSnapshot::Stopped
                } else {
                    AiSnapshot::NotConfigured
                }
            }
        };
        let whisper = state.whisper.session().settings();
        let scheduled = {
            let session = state.windows_actions.session();
            let session = session.lock();
            session.scheduled().len()
        };
        let vault_unlocked = state.notes.is_unlocked();
        let settings = self.settings();
        DesktopSnapshot {
            window_visible,
            ai_state,
            microphone: self.audio.state(),
            vault_unlocked,
            pending_timers: scheduled,
            whisper_configured: whisper.is_configured(),
            vosk_running: matches!(self.audio.state(), MicrophoneState::VoskListening),
            autostart: if settings.autostart_enabled {
                self.autostart_status().state()
            } else {
                AutostartState::Disabled
            },
        }
    }
}

/// The autostart entry, written through the plugin, which uses the supported
/// per-user mechanism and never a shell.
struct PluginAutostart {
    app: AppHandle,
}

impl AutostartBackend for PluginAutostart {
    fn enable(&self, _program: &std::path::Path, _arguments: &[String]) -> Result<(), String> {
        use tauri_plugin_autostart::ManagerExt;
        // The arguments come from the plugin's own configuration (the
        // `--start-minimized` flag), so what is written here is the executable
        // plus that flag and nothing else.
        self.app.autolaunch().enable().map_err(|error| {
            // A policy refusal arrives as a string; it is shortened and carries
            // no path, so it can be shown and logged.
            jarvis_core::text::shorten(&error.to_string(), 120)
        })
    }

    fn disable(&self) -> Result<(), String> {
        use tauri_plugin_autostart::ManagerExt;
        self.app
            .autolaunch()
            .disable()
            .map_err(|error| jarvis_core::text::shorten(&error.to_string(), 120))
    }

    fn status(&self) -> Result<AutostartStatus, String> {
        use tauri_plugin_autostart::ManagerExt;
        let enabled = self
            .app
            .autolaunch()
            .is_enabled()
            .map_err(|error| jarvis_core::text::shorten(&error.to_string(), 120))?;
        Ok(AutostartStatus {
            entry_present: enabled,
            // The plugin reports whether the entry exists; the command it wrote
            // is rewritten on every start, so an entry that exists is an entry
            // that points here.
            path_matches: enabled,
            command: None,
            error: None,
        })
    }
}

/// Builds the tray icon once, from the menu the core module describes.
pub fn install_tray(app: &AppHandle) -> tauri::Result<()> {
    let handle = app.state::<Arc<DesktopHandle>>().inner().clone();
    let snapshot = handle.snapshot(app);
    let menu = build_menu(app, &snapshot)?;
    let tooltip = tray_tooltip(&snapshot);
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip(tooltip)
        // The tray menu is the only menu; a left click opens the window.
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| {
            on_menu_event(app, event.id().as_ref());
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }
    builder.build(app)?;
    Ok(())
}

/// Rebuilds the menu and the tooltip from the current state.
///
/// It is called after every action and after every state poll from the window,
/// so the menu says what is true without a background thread of its own.
pub fn refresh_tray(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let handle = app.state::<Arc<DesktopHandle>>().inner().clone();
    let snapshot = handle.snapshot(app);
    if let Ok(menu) = build_menu(app, &snapshot) {
        let _ = tray.set_menu(Some(menu));
    }
    let _ = tray.set_tooltip(Some(tray_tooltip(&snapshot)));
}

fn build_menu(app: &AppHandle, snapshot: &DesktopSnapshot) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for row in tray_menu(snapshot) {
        if row.separator {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
            continue;
        }
        let label = row_label(&row);
        let item = MenuItem::with_id(app, row.id, label, row.enabled, None::<&str>)?;
        menu.append(&item)?;
    }
    Ok(menu)
}

/// The label of one row: the Fluent key is resolved by the window, so the tray
/// carries a short, stable English word plus its value. The menu is a fallback
/// surface for a hidden window, not a second translation system.
fn row_label(row: &jarvis_core::desktop::TrayMenuRow) -> String {
    let base = english_label(row.label_key);
    match &row.value {
        Some(value) => format!("{base}: {}", english_label(value)),
        None => base.to_string(),
    }
}

/// Short labels for the tray. The window shows the same states translated.
fn english_label(key: &str) -> &str {
    match key {
        "desktop-menu-open" => "Open JARVIS",
        "desktop-menu-focus" => "Open JARVIS (window is shown)",
        "desktop-menu-hide" => "Hide window",
        "desktop-menu-ai" => "AI",
        "desktop-menu-vosk-start" => "Start Vosk",
        "desktop-menu-vosk-stop" => "Stop Vosk",
        "desktop-menu-dictate-start" => "Start dictation",
        "desktop-menu-dictate-stop" => "Stop dictation",
        "desktop-menu-microphone" => "Microphone",
        "desktop-menu-timers" => "Active timers",
        "desktop-menu-lock" => "Lock storage",
        "desktop-menu-settings" => "Settings",
        "desktop-menu-exit" => "Exit",
        "desktop-ai-not-configured" => "not configured",
        "desktop-ai-stopped" => "stopped",
        "desktop-ai-ready" => "ready",
        "desktop-ai-busy" => "working",
        "desktop-mic-idle" => "idle",
        "desktop-mic-vosk" => "Vosk is listening",
        "desktop-mic-whisper" => "recording (dictation)",
        "desktop-mic-file" => "transcribing a file",
        "desktop-mic-stopping" => "stopping",
        "desktop-mic-failed" => "failed",
        other => other,
    }
}

/// What each tray row does.
fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        "open" => show_main_window(app),
        "hide" => hide_main_window(app),
        "vosk_toggle" => toggle_vosk(app),
        "dictate_start" => start_dictation(app),
        "dictate_stop" => {
            // The flag is set and nothing is waited for, so a stop is bounded
            // even when the worker has already died.
            let state = app.state::<AppState>();
            if state.whisper.session().cancel() {
                let _ = jarvis_core::recorder::try_stop_recording();
            }
        }
        "lock_storage" => {
            let state = app.state::<AppState>();
            // The same path the vault page uses, so the journals and the keys go
            // together.
            state.autocorrect.clear_journals();
            state.vault.lock_for_exit();
        }
        "settings" => {
            show_main_window(app);
            let _ = app.emit("desktop-open-settings", ());
        }
        "exit" => request_exit(app),
        // The status rows do nothing.
        _ => {}
    }
    refresh_tray(app);
    let _ = app.emit(STATE_CHANGED_EVENT, ());
}

fn toggle_vosk(app: &AppHandle) {
    let audio = app.state::<Arc<DesktopHandle>>().audio().clone();
    if matches!(audio.state(), MicrophoneState::VoskListening) {
        // Stopping is the session's own business in the voice host; from here
        // the state is released and the host is told to stop.
        audio.release_for_exit();
        log::info!("desktop: Vosk stop requested from the tray");
    } else {
        match audio.begin(MicrophoneState::VoskListening) {
            Ok(_) => log::info!("desktop: Vosk start requested from the tray"),
            Err(_) => log::warn!("desktop: the microphone is already in use"),
        }
    }
}

/// Prepares the recorder in this process, once.
///
/// The window never called `recorder::init()`, so the cells the native read
/// needs were empty and the read panicked inside the worker. The one startup
/// route in `main` prepares the recorder before any command can run; this
/// function stays as the recovery path, and it never hides the reason: the code
/// of the [`jarvis_core::recorder::RecorderError`] is returned unchanged, so the
/// interface says `not_initialized`, `no_input_device`, or `permission_denied`
/// instead of one flattened "audio unavailable".
pub(crate) fn ensure_recorder_ready() -> Result<jarvis_core::recorder::RecorderStatus, String> {
    if !jarvis_core::recorder::is_ready() {
        jarvis_core::recorder::init().map_err(|error| error.code().to_string())?;
    }
    let status = jarvis_core::recorder::status().map_err(|error| error.code().to_string())?;
    if !status.native_ready {
        return Err(jarvis_core::recorder::RecorderError::DeviceFailed(
            "the microphone could not be opened".to_string(),
        )
        .code()
        .to_string());
    }
    // The device list is asked for as well: a backend that opens but lists
    // nothing is the one case `init` cannot see, and it ends the same way.
    jarvis_core::recorder::try_audio_devices().map_err(|error| error.code().to_string())?;
    Ok(status)
}

/// Opens the microphone for a moment and gives it straight back.
///
/// This is the "Проверить микрофон" command: it is the only way to find out
/// whether dictation can work without recording anything. A dictation already in
/// flight owns the device, so it is refused instead of competing for it; every
/// other answer, including a failure, comes back as a value with the recorder's
/// own code.
pub(crate) fn check_microphone(
    session: &jarvis_core::whisper::WhisperSession,
) -> Result<jarvis_core::recorder::MicrophoneCheck, String> {
    if session.holds_microphone() {
        return Err(jarvis_core::recorder::RecorderError::AlreadyRunning
            .code()
            .to_string());
    }
    ensure_recorder_ready()?;
    // Three frames of 512 samples at 16 kHz: about a tenth of a second, enough
    // for a level and short enough that a person does not notice it.
    jarvis_core::recorder::check_microphone(3).map_err(|error| error.code().to_string())
}

fn start_dictation(app: &AppHandle) {
    let audio = app.state::<Arc<DesktopHandle>>().audio().clone();
    let session = Arc::clone(app.state::<AppState>().whisper.session());
    // The microphone has to be usable before the session claims it.
    if let Err(code) = ensure_recorder_ready() {
        log::warn!("desktop: dictation is not possible ({code})");
        return;
    }
    match audio.begin(MicrophoneState::WhisperDictation) {
        Ok(ticket) => {
            // The transcription runs on a worker: the tray must not freeze while
            // a recording is taken.
            std::thread::spawn(move || {
                // Every exit path of this worker releases the microphone: the
                // guard runs on a returned error, on an early exit, and on a
                // panic, which is caught here as the last line of defence. The
                // primary cause is never masked: the typed error from the frame
                // source is what the session reports.
                struct Release {
                    audio: Arc<jarvis_core::desktop::AudioSession>,
                    ticket: jarvis_core::desktop::SessionTicket,
                }
                impl Drop for Release {
                    fn drop(&mut self) {
                        let _ = jarvis_core::recorder::try_stop_recording();
                        self.audio.end(self.ticket);
                    }
                }
                let _release = Release {
                    audio: Arc::clone(&audio),
                    ticket,
                };
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut source = jarvis_core::whisper::RecorderFrames;
                    session.dictate(&mut source)
                }));
                match outcome {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        log::warn!("desktop: dictation failed: {}", error.code())
                    }
                    Err(_) => log::error!(
                        "desktop: the dictation worker panicked; the microphone is released"
                    ),
                }
            });
        }
        Err(_) => log::warn!("desktop: the microphone is already in use"),
    }
}

/// Shows and focuses the main window.
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    refresh_tray(app);
}

/// Hides the window, leaving the process (and the timers) running.
pub fn hide_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    refresh_tray(app);
}

/// Runs the one exit route there is.
pub fn request_exit(app: &AppHandle) {
    let state = app.state::<AppState>();
    let report = state.lifecycle.shutdown();
    if report.is_clean() {
        log::info!("exit: {}", report.summary());
    } else {
        log::warn!("exit incomplete: {}", report.summary());
    }
    app.exit(0);
}

/// What the close button does, decided here and nowhere else.
pub fn on_close_requested(app: &AppHandle, api: &tauri::CloseRequestApi) {
    let close_behavior = app.state::<Arc<DesktopHandle>>().settings().close_behavior;
    match close_behavior {
        CloseBehavior::Tray => {
            api.prevent_close();
            hide_main_window(app);
        }
        CloseBehavior::Exit => {
            api.prevent_close();
            request_exit(app);
        }
        CloseBehavior::Ask => {
            // The window shows the dialog: it can offer "remember", which a
            // native message box cannot.
            api.prevent_close();
            show_main_window(app);
            let _ = app.emit(CLOSE_REQUESTED_EVENT, ());
        }
    }
}

// ------------------------------------------------------------------ commands

#[derive(Clone, Debug, Serialize)]
pub struct DesktopStateView {
    pub settings: DesktopSettings,
    pub setup: SetupState,
    pub microphone: MicrophoneState,
    pub autostart: AutostartStatus,
    pub window_visible: bool,
    pub tray_available: bool,
    pub pending_timers: usize,
    pub ai_state: String,
    pub whisper_configured: bool,
    pub vault_unlocked: bool,
}

#[tauri::command]
pub async fn desktop_get_state(app: AppHandle) -> Result<DesktopStateView, String> {
    let handle = app.state::<Arc<DesktopHandle>>().inner().clone();
    refresh_tray(&app);
    let snapshot = handle.snapshot(&app);
    Ok(DesktopStateView {
        settings: handle.settings(),
        setup: handle.setup(),
        microphone: snapshot.microphone,
        autostart: handle.autostart_status(),
        window_visible: snapshot.window_visible,
        tray_available: app.tray_by_id(TRAY_ID).is_some(),
        pending_timers: snapshot.pending_timers,
        ai_state: snapshot.ai_state.as_str().to_string(),
        whisper_configured: snapshot.whisper_configured,
        vault_unlocked: snapshot.vault_unlocked,
    })
}

#[tauri::command]
pub async fn desktop_show_window(app: AppHandle) -> Result<(), String> {
    show_main_window(&app);
    Ok(())
}

#[tauri::command]
pub async fn desktop_hide_window(app: AppHandle) -> Result<(), String> {
    hide_main_window(&app);
    Ok(())
}

/// The full exit, used by the tray, the close dialog, and the settings page.
#[tauri::command]
pub async fn desktop_request_exit(app: AppHandle) -> Result<(), String> {
    request_exit(&app);
    Ok(())
}

#[tauri::command]
pub async fn desktop_get_close_behavior(
    state: tauri::State<'_, Arc<DesktopHandle>>,
) -> Result<CloseBehavior, String> {
    Ok(state.settings().close_behavior)
}

/// Sets what the close button does, and remembers it when the user asked to.
#[tauri::command]
pub async fn desktop_set_close_behavior(
    app: AppHandle,
    state: tauri::State<'_, Arc<DesktopHandle>>,
    behavior: CloseBehavior,
    remember: bool,
) -> Result<DesktopSettings, String> {
    let handle = state.inner().clone();
    let mut settings = handle.settings();
    if remember {
        settings.close_behavior = behavior;
    }
    settings.tray_explained = true;
    let updated = handle.update_settings(settings)?;
    refresh_tray(&app);
    Ok(updated)
}

/// Stores the desktop settings: the close behaviour and the four switches that
/// decide what a login starts. The core writes the document and applies what can
/// be applied (the autostart entry).
#[tauri::command]
pub async fn desktop_update_settings(
    app: AppHandle,
    settings: DesktopSettings,
) -> Result<DesktopSettings, String> {
    let handle = app.state::<Arc<DesktopHandle>>().inner().clone();
    let updated = handle.update_settings(settings)?;
    refresh_tray(&app);
    Ok(updated)
}

/// Finds a Whisper build that is already on this machine.
///
/// Nothing is saved: the search only reports what passed the same validation the
/// settings page uses, and the user confirms which pair to use.
#[tauri::command]
pub async fn whisper_discover() -> Result<jarvis_core::whisper::DiscoveryReport, String> {
    Ok(jarvis_core::whisper::discover())
}

/// Stores a discovered pair, after the user confirmed it.
#[tauri::command]
pub async fn whisper_apply_discovered(
    state: tauri::State<'_, AppState>,
    executable: String,
    model: String,
) -> Result<WhisperSettings, String> {
    // The pair is validated again here: a path that arrived from the window is
    // not trusted, even when the search proposed it.
    jarvis_core::whisper::probe_binary(std::path::Path::new(&executable))
        .map_err(|error| error.to_string())?;
    jarvis_core::whisper::probe_model(std::path::Path::new(&model))
        .map_err(|error| error.to_string())?;
    let session = state.whisper.session();
    let mut settings = session.settings();
    settings.binary_path = executable;
    settings.model_path = model;
    let updated = session
        .update_settings(settings)
        .map_err(|error| error.to_string())?;
    session.set_program(std::path::Path::new(&updated.binary_path));
    Ok(updated)
}

/// The typed action that locks the encrypted storages, from the tray or the
/// window. It goes through the same session and the same audit as any action.
#[tauri::command]
pub async fn desktop_lock_storage(app: AppHandle) -> Result<bool, String> {
    let state = app.state::<AppState>();
    state.autocorrect.clear_journals();
    state.vault.lock_for_exit();
    Ok(true)
}

#[tauri::command]
pub async fn autostart_get_state(
    state: tauri::State<'_, Arc<DesktopHandle>>,
) -> Result<AutostartStatus, String> {
    Ok(state.autostart_status())
}

#[tauri::command]
pub async fn autostart_enable(
    state: tauri::State<'_, Arc<DesktopHandle>>,
) -> Result<AutostartStatus, String> {
    state.autostart_enable()
}

#[tauri::command]
pub async fn autostart_disable(
    state: tauri::State<'_, Arc<DesktopHandle>>,
) -> Result<AutostartStatus, String> {
    state.autostart_disable()
}

#[tauri::command]
pub async fn setup_get_state(
    state: tauri::State<'_, Arc<DesktopHandle>>,
) -> Result<SetupState, String> {
    Ok(state.setup())
}

#[tauri::command]
pub async fn setup_complete_step(
    state: tauri::State<'_, Arc<DesktopHandle>>,
    step: String,
    language: Option<String>,
) -> Result<SetupState, String> {
    let handle = state.inner().clone();
    let mut setup = handle.setup();
    setup
        .complete_step(&step)
        .map_err(|error| error.to_string())?;
    if let Some(language) = language {
        setup.language = Some(language);
    }
    handle.save_setup(&setup)?;
    Ok(setup)
}

#[tauri::command]
pub async fn setup_skip_step(
    state: tauri::State<'_, Arc<DesktopHandle>>,
    step: String,
) -> Result<SetupState, String> {
    let handle = state.inner().clone();
    let mut setup = handle.setup();
    setup.skip_step(&step).map_err(|error| error.to_string())?;
    handle.save_setup(&setup)?;
    Ok(setup)
}

#[tauri::command]
pub async fn setup_finish(
    state: tauri::State<'_, Arc<DesktopHandle>>,
    timestamp: String,
) -> Result<SetupState, String> {
    let handle = state.inner().clone();
    let mut setup = handle.setup();
    setup.finish(timestamp);
    handle.save_setup(&setup)?;
    Ok(setup)
}

/// Runs the wizard again without touching any setting.
#[tauri::command]
pub async fn setup_reset(
    state: tauri::State<'_, Arc<DesktopHandle>>,
) -> Result<SetupState, String> {
    let handle = state.inner().clone();
    let mut setup = handle.setup();
    setup.reset();
    handle.save_setup(&setup)?;
    Ok(setup)
}

/// The directory the application keeps its own files in.
pub fn data_directory() -> PathBuf {
    jarvis_core::notes::vault::VaultPaths::production()
        .map(|paths| paths.data_dir)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn current_executable() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("jarvis-app.exe"))
}
