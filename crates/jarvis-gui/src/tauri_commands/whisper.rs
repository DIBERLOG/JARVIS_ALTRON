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
    ///
    /// It is shared, not copied, and that is the whole point: every command
    /// clones the handle, so a transcript written by the dictation command has
    /// to be the one the status command reads. `Clone` used to make a *copy* of
    /// this mutex, and the text was remembered into a copy the command dropped
    /// on its way out — the backend logged `result_delivered` and the window was
    /// given nothing.
    last: Arc<parking_lot::Mutex<Option<Transcript>>>,
    /// How many characters the window was last given, or -1 for "nothing yet".
    /// It exists so the one line that says the interface really has the result
    /// is logged when it changes, not on every poll.
    reported: Arc<std::sync::atomic::AtomicI64>,
}

impl Clone for WhisperHandle {
    fn clone(&self) -> Self {
        Self {
            session: Arc::clone(&self.session),
            last: Arc::clone(&self.last),
            reported: Arc::clone(&self.reported),
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
            last: Arc::new(parking_lot::Mutex::new(None)),
            reported: Arc::new(std::sync::atomic::AtomicI64::new(-1)),
        }
    }

    pub fn session(&self) -> &Arc<WhisperSession> {
        &self.session
    }

    /// Remembers a transcript for the window to show, replacing the previous one.
    pub fn remember(&self, transcript: Transcript) {
        *self.last.lock() = Some(transcript);
    }

    /// Forgets the transcript. This is the only way it goes away.
    pub fn forget(&self) {
        *self.last.lock() = None;
    }

    pub fn last(&self) -> Option<Transcript> {
        self.last.lock().clone()
    }

    /// The transcript as the window's own state, with the one line that says the
    /// interface is being given it.
    ///
    /// The line is logged when the answer changes — never the text, only whether
    /// there is one and how long it is. `result_delivered` on its own is not
    /// proof: it says the command handed the text over, and this says the window
    /// is actually being served it.
    pub fn last_for_window(&self) -> Option<Transcript> {
        let last = self.last();
        let characters = match &last {
            Some(transcript) => transcript.text.chars().count() as i64,
            None => -1,
        };
        if self
            .reported
            .swap(characters, std::sync::atomic::Ordering::SeqCst)
            != characters
        {
            if characters < 0 {
                log::info!("whisper: ui_result_available=false characters=0");
            } else {
                log::info!("whisper: ui_result_available=true characters={characters}");
            }
        }
        last
    }

    /// Stops anything that is running, for a full exit. Safe to repeat.
    pub fn shutdown(&self) {
        self.session.shutdown();
        self.forget();
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
            last: handle.last_for_window(),
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
///
/// This is the only thing that clears it: a state poll, a page switch, and a
/// new dictation all leave the last text in place, and a new result replaces it.
#[tauri::command]
pub async fn whisper_clear_last(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let handle = state.whisper.clone();
    on_blocking(move || {
        handle.forget();
        log::info!("whisper: ui_result_available=false characters=0 source=clear");
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A handle over a directory of its own, without touching the real profile.
    fn handle() -> WhisperHandle {
        let directory = std::env::temp_dir().join("jarvis-gui-whisper-handle");
        let _ = std::fs::create_dir_all(&directory);
        WhisperHandle {
            session: Arc::new(WhisperSession::open(&directory, WhisperSettings::default())),
            last: Arc::new(parking_lot::Mutex::new(None)),
            reported: Arc::new(std::sync::atomic::AtomicI64::new(-1)),
        }
    }

    fn transcript(text: &str) -> Transcript {
        Transcript {
            text: text.to_string(),
            segments: Vec::new(),
            language: "ru".to_string(),
            audio_ms: 1_000,
            duration_ms: 10,
        }
    }

    /// The defect this whole change was about: `Clone` made a *copy* of the
    /// transcript, so the dictation command remembered the text into a copy the
    /// status command never saw. The backend logged `result_delivered` and the
    /// window was given nothing.
    #[test]
    fn a_transcript_remembered_through_a_clone_is_read_by_another_clone() {
        let handle = handle();
        // The status command starts from its own clone, and there is nothing yet.
        assert!(handle.clone().last_for_window().is_none());

        // The dictation command remembers through a clone.
        let dictate = handle.clone();
        dictate.remember(transcript("FICTIONAL_SECRET"));

        // The status command asks through yet another clone: it must see the text.
        let served = handle
            .clone()
            .last_for_window()
            .expect("the window must be served the text the dictation delivered");
        assert_eq!(served.text, "FICTIONAL_SECRET");
        assert_eq!(served.language, "ru");
        // And the original handle has it too: one value, one place.
        assert_eq!(handle.last().unwrap().text, "FICTIONAL_SECRET");
    }

    /// Clearing is the only thing that removes it, and it removes it everywhere.
    #[test]
    fn only_clearing_removes_the_transcript_and_it_does_so_once() {
        let handle = handle();
        handle.remember(transcript("FICTIONAL_SECRET"));
        assert!(handle.last_for_window().is_some());
        // Other clones keep seeing it: nothing drops it behind the panel's back.
        for _ in 0..3 {
            assert!(handle.clone().last_for_window().is_some());
        }
        handle.clone().forget();
        assert!(handle.last_for_window().is_none());
        assert!(handle.clone().last_for_window().is_none());
    }

    /// A new result replaces the previous one, and it is the new one that is served.
    #[test]
    fn a_new_transcript_replaces_the_previous_one() {
        let handle = handle();
        handle.remember(transcript("first"));
        handle.clone().remember(transcript("second"));
        assert_eq!(handle.last_for_window().unwrap().text, "second");
    }
}
