//! Dictation commands exposed to the interface.
//!
//! Rules that shape this file:
//!
//! * the window never runs a process and never names a model path: it asks, and
//!   the core opens the native file dialog and validates what was picked;
//! * a dictation records only between an explicit start and an explicit stop.
//!   Nothing here can leave a microphone open: the core refuses a second job and
//!   the switch in the settings stops the work in flight;
//! * the transcript is returned to the window and shown to the person who
//!   dictated it. It is not logged, not written to the AI memory, and not sent
//!   to the local model by anything in this module;
//! * every command runs on the blocking pool, so a transcription never freezes
//!   the window.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use tauri::Emitter;

use jarvis_core::notes::vault::VaultPaths;
use jarvis_core::recorder::MicrophoneCheck;
use jarvis_core::whisper::{
    DictationStatus, RecorderFrames, Transcript, WhisperError, WhisperSession, WhisperSettings,
};

use crate::AppState;

/// The dictation session plus the transcript the window last received.
pub struct WhisperHandle {
    session: Arc<WhisperSession>,
    /// Kept in memory only, so the panel can still show the text after a
    /// reload. Nothing writes it to disk.
    last: parking_lot::Mutex<Option<Transcript>>,
}

impl Clone for WhisperHandle {
    fn clone(&self) -> Self {
        Self {
            session: Arc::clone(&self.session),
            last: parking_lot::Mutex::new(self.last.lock().clone()),
        }
    }
}

impl WhisperHandle {
    /// Opens dictation over the application data directory.
    pub fn restore() -> Self {
        let directory = data_directory();
        let settings = WhisperSettings::default();
        let session = WhisperSession::open(&directory, settings);
        // The session uses the executable the user picked, if there is one.
        let stored = session.settings();
        if !stored.binary_path.is_empty() {
            session.set_program(std::path::Path::new(&stored.binary_path));
        }
        Self {
            session: Arc::new(session),
            last: parking_lot::Mutex::new(None),
        }
    }

    pub fn session(&self) -> &Arc<WhisperSession> {
        &self.session
    }

    /// Remembers a transcript for the window to show.
    pub fn remember(&self, transcript: Transcript) {
        *self.last.lock() = Some(transcript);
    }

    pub fn last(&self) -> Option<Transcript> {
        self.last.lock().clone()
    }

    /// Stops anything that is running, for a full exit. Safe to repeat.
    pub fn shutdown(&self) {
        self.session.shutdown();
        *self.last.lock() = None;
    }
}

impl std::fmt::Debug for WhisperHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WhisperHandle")
            .field("state", &self.session.state())
            .field("configured", &self.session.settings().is_configured())
            .finish()
    }
}

/// The state the panel shows, together with the text it last received.
#[derive(Clone, Debug, Serialize)]
pub struct WhisperPanelView {
    pub status: DictationStatus,
    pub settings: WhisperSettings,
    pub last: Option<Transcript>,
}

#[tauri::command]
pub async fn whisper_status(state: tauri::State<'_, AppState>) -> Result<WhisperPanelView, String> {
    let handle = state.whisper.clone();
    on_blocking(move || {
        Ok(WhisperPanelView {
            status: handle.session().status(),
            settings: handle.session().settings(),
            last: handle.last(),
        })
    })
    .await
}

/// Stores the settings. A path that is set but unusable is refused here, before
/// anything is started.
#[tauri::command]
pub async fn whisper_update_settings(
    state: tauri::State<'_, AppState>,
    settings: WhisperSettings,
) -> Result<WhisperSettings, String> {
    let handle = state.whisper.clone();
    on_blocking(move || {
        let updated = handle
            .session()
            .update_settings(settings)
            .map_err(describe)?;
        if !updated.binary_path.is_empty() {
            handle
                .session()
                .set_program(std::path::Path::new(&updated.binary_path));
        }
        Ok(updated)
    })
    .await
}

/// Opens the native file dialog for the executable and stores what was picked.
#[tauri::command]
pub async fn whisper_select_binary(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<WhisperSettings>, String> {
    let Some(path) = pick(&app, "exe") else {
        return Ok(None);
    };
    let handle = state.whisper.clone();
    on_blocking(move || {
        let mut settings = handle.session().settings();
        settings.binary_path = path.to_string_lossy().into_owned();
        let updated = handle
            .session()
            .update_settings(settings)
            .map_err(describe)?;
        handle
            .session()
            .set_program(std::path::Path::new(&updated.binary_path));
        Ok(Some(updated))
    })
    .await
}

/// Opens the native file dialog for the model and stores what was picked.
#[tauri::command]
pub async fn whisper_select_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<WhisperSettings>, String> {
    let Some(path) = pick(&app, "bin") else {
        return Ok(None);
    };
    let handle = state.whisper.clone();
    on_blocking(move || {
        let mut settings = handle.session().settings();
        settings.model_path = path.to_string_lossy().into_owned();
        let updated = handle
            .session()
            .update_settings(settings)
            .map_err(describe)?;
        Ok(Some(updated))
    })
    .await
}

/// Records from the microphone and returns the text.
///
/// The recording ends on silence, on the configured length, or when the window
/// asks to stop it.
#[tauri::command]
pub async fn whisper_dictate(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Transcript, String> {
    let handle = state.whisper.clone();
    on_blocking(move || {
        // The recorder is prepared in the startup route; this call is the
        // recovery path for a machine where that first attempt failed. The code
        // itself is what the window receives — `not_initialized`,
        // `no_input_device`, `permission_denied` — so the panel names the real
        // cause instead of showing one flattened "audio unavailable".
        crate::desktop::ensure_recorder_ready().map_err(|code| {
            log::warn!("whisper: recorder_unavailable ({code})");
            code
        })?;
        let mut source = RecorderFrames::new();
        // A recorder failure travels as its own code, so the panel shows
        // `not_initialized`, `recorder_busy`, `vosk_owns_microphone`,
        // `start_failed` — and not one sentence for all of them. Every other
        // failure travels as its code too, because the panel translates a bare
        // code and a sentence in English is not an answer a Russian window can
        // show. The log keeps the stage as well.
        let transcript = match handle.session().dictate(&mut source) {
            Ok(transcript) => transcript,
            Err(error) => {
                return match recorder_failure(&error) {
                    Some((stage, code)) => {
                        log::warn!(
                            "whisper: {} (stage={stage} recorder_code={code})",
                            error.code()
                        );
                        Err(code.to_string())
                    }
                    None => Err(describe(error)),
                };
            }
        };
        // The result is delivered to the window: remembered in the session,
        // where the panel reads it, and announced so it does not have to wait
        // for its next poll. The text is not in the event.
        let characters = transcript.text.chars().count();
        handle.remember(transcript.clone());
        log::info!("whisper: stage=result_delivered characters={characters} source=window");
        let _ = app.emit(crate::desktop::WHISPER_TRANSCRIPT_EVENT, ());
        Ok(transcript)
    })
    .await
}

/// Checks the microphone without recording anything.
///
/// This is the diagnostic the settings page offers: the device is opened, a few
/// frames are read so a signal level can be reported, and the device is released
/// again on every path, including a failure. Nothing is written, nothing is sent
/// to whisper, and no audio is kept: the answer is the recorder status, the
/// number of frames that were read, and a level between 0 and 1.
#[tauri::command]
pub async fn whisper_check_microphone(
    state: tauri::State<'_, AppState>,
) -> Result<MicrophoneCheck, String> {
    let session = std::sync::Arc::clone(state.whisper.session());
    on_blocking(move || crate::desktop::check_microphone(&session)).await
}

/// Transcribes an audio file the user picks.
///
/// The file must already be 16 kHz mono 16-bit PCM; another format is reported
/// rather than converted.
#[tauri::command]
pub async fn whisper_transcribe_file(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<Transcript>, String> {
    let Some(path) = pick(&app, "wav") else {
        return Ok(None);
    };
    let handle = state.whisper.clone();
    on_blocking(move || {
        let transcript = handle.session().transcribe_path(&path).map_err(describe)?;
        handle.remember(transcript.clone());
        Ok(Some(transcript))
    })
    .await
}

/// Stops a recording or a transcription. Returns whether anything was running.
#[tauri::command]
pub async fn whisper_cancel(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    let handle = state.whisper.clone();
    on_blocking(move || Ok(handle.session().cancel())).await
}

/// Forgets the transcript the panel was showing.
#[tauri::command]
pub async fn whisper_clear_last(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let handle = state.whisper.clone();
    on_blocking(move || {
        *handle.last.lock() = None;
        Ok(())
    })
    .await
}

// ----------------------------------------------------------------------------- helpers

async fn on_blocking<T, F>(action: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(action).await {
        Ok(result) => result,
        Err(_) => Err(describe(WhisperError::Storage)),
    }
}

/// The application data directory, from the same paths the storage uses.
fn data_directory() -> PathBuf {
    VaultPaths::production()
        .map(|paths| paths.data_dir)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// A message that is safe to show and to log: the error codes carry no path and
/// no transcript.
///
/// The *code* is what the window receives, not the English sentence: the panel
/// translates a bare code, and every code the core can produce has a message in
/// all three locales. The detail — the one part that can be a sentence — is
/// logged, where it is a diagnosis rather than the user's answer.
fn describe(error: WhisperError) -> String {
    match &error {
        // The stage and the recorder's own code are what makes this
        // diagnosable: `recorder_unavailable` on its own hid a missing start.
        WhisperError::RecorderUnavailable { stage, code } => log::warn!(
            "whisper: {} (stage={stage} recorder_code={code})",
            error.code()
        ),
        _ => match error.detail() {
            Some(detail) => log::warn!("whisper: {} (detail={detail})", error.code()),
            None => log::warn!("whisper: {}", error.code()),
        },
    }
    error.code().to_string()
}

/// The recorder's own stage and code, when the failure came from the recorder.
fn recorder_failure(error: &WhisperError) -> Option<(&'static str, &'static str)> {
    match error {
        WhisperError::RecorderUnavailable { stage, code } => Some((stage, code)),
        _ => None,
    }
}

fn pick(app: &tauri::AppHandle, extension: &str) -> Option<PathBuf> {
    use tauri_plugin_dialog::DialogExt;
    app.dialog()
        .file()
        .set_title("JARVIS")
        .add_filter("Whisper", &[extension])
        .blocking_pick_file()
        .and_then(|path| path.into_path().ok())
}
