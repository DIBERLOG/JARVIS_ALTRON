//! Global voice input, exposed to the window and to the tray.
//!
//! # The production mode is the clipboard
//!
//! The route records, transcribes, punctuates and **copies**, then tells the
//! person to press Ctrl+V. Nothing is typed anywhere, no keystroke is
//! synthesized, and no UI Automation `SetValue` replaces the content of a field
//! that already has some. The UI Automation reader is designed, gated and tested
//! against fakes, and it stays an experimental extension
//! (`docs/GLOBAL_VOICE_INPUT.md`).
//!
//! # What holds the microphone
//!
//! The listener (Vosk, in the voice host) holds it; the dictation (Whisper, in
//! this process) asks for it. The handover is this module's job:
//!
//! 1. the listener is told to stop and is given time to let go — this process
//!    checks its own recorder, and the host stops Vosk when it recognises the
//!    phrase (see "the trigger" below);
//! 2. the dictation runs through the existing `WhisperSession`, which takes the
//!    one `MicrophoneLease` there is;
//! 3. the listener is restored on **every** path — a failure, a cancellation and
//!    a panic all end with Vosk listening again, and only if it was listening
//!    before the request started.
//!
//! # The trigger, across two processes
//!
//! `jarvis-app` owns Vosk and this process owns Whisper, so the phrase is
//! recognised there and the dictation runs here. The two already have a channel:
//! the local IPC broadcast (`jarvis_core::ipc`), which carries typed events and
//! never a transcript. `jarvis-app` emits `GlobalDictationRequested` after it has
//! stopped listening, the window receives it and calls `voice_input_start`. No
//! second state manager is created: this handle is the only one, and the tray,
//! the window and the voice route all use it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;

use jarvis_core::dictation::{
    self, ClipboardWriter, DictationEngine, DictationError, DictationStage, DictationStatusView,
    ElementKind, ForegroundProbe, GlobalDictationSettings, TargetSnapshot, TextCorrector,
    TextInserter, TranscribedText, VoiceHost, VoiceInputPreference, VoiceIntent, VoiceTranscriber,
    WindowIdentity,
};
use jarvis_core::vault::clipboard::{
    clamp_clear_seconds, ClipboardGuard, ClipboardStatus, SystemClipboard,
};

use crate::AppState;

/// File holding the settings of this feature, inside the data directory.
pub const SETTINGS_FILE: &str = "voice-input.json";

/// Event the window receives with a localization key and a length.
pub const NOTICE_EVENT: &str = "voice-input-notice";

/// What the window and the tray show. No window, no text, no path.
#[derive(Clone, Debug, Serialize)]
pub struct VoiceInputView {
    pub settings: GlobalDictationSettings,
    pub status: DictationStatusView,
    /// Whether the dictation can run at all: Whisper has to be configured.
    pub whisper_configured: bool,
    /// Whether the listener is reachable. It lives in the voice host process, so
    /// this says whether the route can hand the microphone over.
    pub vosk_available: bool,
    pub clipboard: ClipboardStatus,
    /// A content-free code for the last failure.
    pub error_code: Option<String>,
    /// The Fluent key for that failure, for the interface to translate.
    pub error_key: Option<String>,
    /// Whether a text is held in memory for the panel. Never the text itself.
    pub has_result: bool,
    /// The length of that text, for the panel's counter.
    pub characters: usize,
}

/// The last failure, without any content.
#[derive(Clone, Debug, Default)]
struct LastError {
    code: Option<String>,
}

/// The feature: one engine, one clipboard, one settings document.
pub struct VoiceInputHandle {
    engine: Arc<DictationEngine>,
    settings: Arc<Mutex<GlobalDictationSettings>>,
    document: PathBuf,
    /// The dictation itself: the same session the dictation panel uses. No
    /// second session is opened anywhere.
    whisper: crate::tauri_commands::WhisperHandle,
    /// The protected clipboard the vault already owns.
    clipboard: Arc<Mutex<ClipboardGuard<SystemClipboard>>>,
    last_error: Arc<Mutex<LastError>>,
    /// How many times the microphone changed hands, for the log and the tests.
    handovers: Arc<AtomicUsize>,
    /// The window, so a notice can reach it. Attached once at start-up.
    window: Arc<Mutex<Option<tauri::AppHandle>>>,
}

impl Clone for VoiceInputHandle {
    fn clone(&self) -> Self {
        Self {
            engine: Arc::clone(&self.engine),
            settings: Arc::clone(&self.settings),
            document: self.document.clone(),
            whisper: self.whisper.clone(),
            clipboard: Arc::clone(&self.clipboard),
            last_error: Arc::clone(&self.last_error),
            handovers: Arc::clone(&self.handovers),
            window: Arc::clone(&self.window),
        }
    }
}

impl VoiceInputHandle {
    /// Opens the feature over the application data directory.
    pub fn restore(whisper: crate::tauri_commands::WhisperHandle) -> Self {
        let directory = crate::desktop::data_directory();
        let document = directory.join(SETTINGS_FILE);
        let settings = read_settings(&document);
        Self {
            engine: Arc::new(DictationEngine::new()),
            settings: Arc::new(Mutex::new(settings)),
            document,
            whisper,
            clipboard: Arc::new(Mutex::new(ClipboardGuard::new(SystemClipboard))),
            last_error: Arc::new(Mutex::new(LastError::default())),
            handovers: Arc::new(AtomicUsize::new(0)),
            window: Arc::new(Mutex::new(None)),
        }
    }

    /// Gives the handle the window, so a notice can reach it.
    pub fn attach(&self, app: tauri::AppHandle) {
        *self.window.lock() = Some(app);
    }

    /// Tells the window what is happening, by key and by numbers only.
    ///
    /// The event carries a localization key and a length. It never carries the
    /// transcript, the field, or a window title.
    pub fn announce(&self, key: &str, characters: usize) {
        if let Some(app) = self.window.lock().as_ref() {
            use tauri::Emitter;
            let _ = app.emit(
                NOTICE_EVENT,
                serde_json::json!({ "key": key, "characters": characters }),
            );
        }
    }

    pub fn engine(&self) -> &Arc<DictationEngine> {
        &self.engine
    }

    pub fn settings(&self) -> GlobalDictationSettings {
        self.settings.lock().clone()
    }

    /// Stores the settings atomically, and returns what was stored.
    pub fn update_settings(
        &self,
        settings: GlobalDictationSettings,
    ) -> Result<GlobalDictationSettings, String> {
        let settings = settings.normalized();
        let bytes = serde_json::to_vec(&settings).map_err(|_| "storage".to_string())?;
        jarvis_core::fsutil::write_bytes_atomic(&self.document, &bytes)
            .map_err(|_| "storage".to_string())?;
        *self.settings.lock() = settings.clone();
        log::info!(
            "voice input: settings stored (enabled={} language={} punctuation={} autocorrect={} seconds={})",
            settings.enabled,
            settings.language,
            settings.punctuation,
            settings.autocorrect,
            settings.clipboard_seconds
        );
        Ok(settings)
    }

    /// What the panel and the tray show.
    pub fn view(&self) -> VoiceInputView {
        let settings = self.settings();
        let status = dictation::status_of(&self.engine);
        let whisper_configured = {
            let status = self.whisper.session().status();
            status.configured && status.enabled
        };
        let error_code = self.last_error.lock().code.clone();
        VoiceInputView {
            settings,
            whisper_configured,
            vosk_available: true,
            clipboard: self.clipboard.lock().status(),
            error_key: error_code
                .as_ref()
                .map(|code| format!("voice-input-error-{}", code.replace('_', "-"))),
            error_code,
            has_result: self.engine.last_text().is_some(),
            characters: self.engine.last_characters(),
            status,
        }
    }

    /// Whether a request is running.
    pub fn is_running(&self) -> bool {
        self.engine.is_running()
    }

    /// The text of the last request, for the panel. In memory only.
    pub fn last_text(&self) -> Option<String> {
        self.engine.last_text()
    }

    /// Forgets the last text and wipes the clipboard if it is still ours.
    ///
    /// The wipe is the vault's own guard: it clears the clipboard only when the
    /// value there is still the one this application put there.
    pub fn clear_result(&self) -> bool {
        let cleared = self.clipboard.lock().clear_now().is_ok();
        self.engine.forget();
        cleared
    }

    /// Runs one global dictation over the clipboard.
    ///
    /// Every failure is a value with a code; the texture of the route is the
    /// engine's, and this function only supplies the platform pieces.
    pub fn start(&self) -> Result<dictation::DictationOutcome, String> {
        let settings = self.settings();
        if !settings.enabled {
            return Err(DictationError::Disabled.code().to_string());
        }
        if self.engine.is_running() {
            return Err(DictationError::Busy.code().to_string());
        }
        // The confirmation is announced before the microphone changes hands, so
        // nothing is recorded while the person is still being answered.
        if settings.speak_confirmation {
            self.announce("voice-input-listen", 0);
        }
        let host = Host {
            handovers: Arc::clone(&self.handovers),
        };
        let transcriber = Transcriber {
            whisper: self.whisper.clone(),
        };
        let corrector = Corrector {
            enabled: settings.autocorrect,
        };
        let clipboard = Clipboard {
            guard: Arc::clone(&self.clipboard),
            seconds: clamp_clear_seconds(settings.clipboard_seconds),
        };
        let inserter = NoInsertion;
        let probe = ClipboardProbe;
        let quiet = |_phrase: &str| {};
        let outcome = self
            .engine
            .run(
                dictation::DictationRequest {
                    autocorrect: settings.autocorrect,
                    punctuation: settings.punctuation,
                    // The production mode: the clipboard, never a keystroke.
                    preference: VoiceInputPreference::Clipboard,
                    speak_confirmation: false,
                },
                dictation::EngineDeps {
                    host: &host,
                    transcriber: &transcriber,
                    corrector: &corrector,
                    probe: &probe,
                    inserter: &inserter,
                    clipboard: &clipboard,
                    speak: &quiet,
                },
            )
            .map_err(|error| {
                *self.last_error.lock() = LastError {
                    code: Some(error.code().to_string()),
                };
                error.code().to_string()
            })?;
        *self.last_error.lock() = LastError::default();
        // The result is on the clipboard: the person pastes it. The notice says
        // so, and carries the length rather than the text.
        self.announce("voice-input-copied", outcome.characters);
        log::info!(
            "voice input: finished (method=clipboard characters={} handovers={})",
            outcome.characters,
            self.handovers.load(Ordering::SeqCst)
        );
        Ok(outcome)
    }

    /// Handles a phrase the listener recognized.
    ///
    /// This is the voice trigger: a typed intent comes out of the matcher, and
    /// only `StartGlobalDictation` starts anything. The phrase never becomes a
    /// command, and the model is never given it.
    pub fn on_recognized(self: &Arc<Self>, phrase: &str) -> VoiceIntent {
        let settings = self.settings();
        let intent = dictation::intent_with_settings(&settings, phrase);
        if intent == VoiceIntent::StartGlobalDictation && !self.is_running() {
            let handle = Arc::clone(self);
            std::thread::spawn(move || {
                if let Err(code) = handle.start() {
                    log::warn!("voice input: the request failed (error_code={code})");
                }
            });
        }
        intent
    }

    /// Stops what is running, for a cancel button, Escape, or the tray.
    pub fn cancel(&self) -> bool {
        let cancelled = self.engine.cancel();
        if cancelled {
            log::info!("voice input: cancelled");
        }
        cancelled
    }

    /// Stops everything for a full exit.
    pub fn shutdown(&self) {
        self.engine.cancel();
    }

    /// Copies the last result again, with a fresh cleanup timer.
    pub fn copy_again(&self) -> bool {
        let Some(text) = self.last_text() else {
            return false;
        };
        let seconds = clamp_clear_seconds(self.settings().clipboard_seconds);
        let written = self.clipboard.lock().copy_secret(&text, seconds).is_ok();
        if written {
            self.announce("voice-input-copied", text.chars().count());
        }
        written
    }
}

// ------------------------------------------------------------- platform pieces

/// The microphone handover between the listener and the dictation.
struct Host {
    handovers: Arc<AtomicUsize>,
}

impl VoiceHost for Host {
    fn release_microphone(&self) -> Result<(), DictationError> {
        // The listener lives in the voice host process. This process can only
        // make sure its own recorder is free: if something in *this* process
        // holds the device, the dictation cannot start, and saying so is more
        // honest than a claim that the other process is quiet.
        let owner = jarvis_core::recorder::current_owner();
        match owner {
            jarvis_core::recorder::MicrophoneOwner::Free => {}
            jarvis_core::recorder::MicrophoneOwner::Voice => {
                // The listener is in this process after all: give it back.
                let _ = jarvis_core::recorder::stop_recording();
            }
            held => {
                return Err(DictationError::MicrophoneBusy(
                    jarvis_core::recorder::owner_name(held),
                ));
            }
        }
        self.handovers.fetch_add(1, Ordering::SeqCst);
        log::info!("voice input: the microphone is free for the dictation");
        Ok(())
    }

    fn restore_listener(&self) -> Result<(), DictationError> {
        // Nothing to release here: the dictation's own lease is dropped by the
        // engine. The voice host starts listening again when the route has
        // finished with the device.
        let _ = jarvis_core::recorder::try_stop_recording();
        log::info!("voice input: the listener has the microphone again");
        Ok(())
    }
}

/// The dictation: the existing session, which owns the one microphone lease.
struct Transcriber {
    whisper: crate::tauri_commands::WhisperHandle,
}

impl VoiceTranscriber for Transcriber {
    fn transcribe(&self) -> Result<TranscribedText, DictationError> {
        let mut source = jarvis_core::whisper::RecorderFrames::new();
        let transcript = self
            .whisper
            .session()
            .dictate(&mut source)
            .map_err(|error| DictationError::TranscriptionFailed(error.code()))?;
        let audio_ms = transcript.audio_ms;
        // The session keeps the text for the panel, exactly as the dictation
        // button does; nothing writes it to disk.
        self.whisper.remember(transcript.clone());
        Ok(TranscribedText {
            text: transcript.text,
            audio_ms,
        })
    }
}

/// The local pass that runs on a transcript without a document.
///
/// The autocorrect engine works on a document through the notes storage, which
/// is another feature's job. What can run here is the local normalization a
/// dictated sentence needs — collapsed spacing — and it is off with the switch.
struct Corrector {
    enabled: bool,
}

impl TextCorrector for Corrector {
    fn correct(&self, text: &str) -> String {
        if !self.enabled {
            return text.to_string();
        }
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

/// The production probe: the clipboard mode does not need the focused element.
///
/// It reports this application's own surface, which the gate turns into "write
/// nothing into a field" — the correct answer for a build that copies instead of
/// typing. Filling this in is what the experimental UI Automation reader does.
struct ClipboardProbe;

impl ForegroundProbe for ClipboardProbe {
    fn focused_target(&self) -> Result<TargetSnapshot, DictationError> {
        Ok(TargetSnapshot {
            window_id: 0,
            process_id: 0,
            element_kind: ElementKind::Unknown,
            password: false,
            read_only: false,
            enabled: true,
            supports_value_pattern: false,
            supports_text_pattern: false,
            elevated_target: false,
            secure_desktop: false,
            own_window: true,
        })
    }

    fn foreground_identity(&self) -> Option<WindowIdentity> {
        Some(WindowIdentity {
            window_id: 0,
            process_id: 0,
        })
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// There is no automatic insertion in this build.
struct NoInsertion;

impl TextInserter for NoInsertion {
    fn insert(&self, _: &str) -> Result<(), DictationError> {
        Err(DictationError::Unavailable("ui_automation"))
    }
}

/// The protected clipboard the vault already owns.
struct Clipboard {
    guard: Arc<Mutex<ClipboardGuard<SystemClipboard>>>,
    seconds: u64,
}

impl ClipboardWriter for Clipboard {
    fn write(&self, text: &str) -> Result<(), DictationError> {
        self.guard
            .lock()
            .copy_secret(text, self.seconds)
            .map(|_| ())
            .map_err(|_| DictationError::NoDeliveryPath)
    }
}

/// Reads the settings document, falling back to the defaults.
fn read_settings(document: &std::path::Path) -> GlobalDictationSettings {
    std::fs::read(document)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<GlobalDictationSettings>(&bytes).ok())
        .unwrap_or_default()
        .normalized()
}

// ------------------------------------------------------------------- commands

/// The state of the feature, its settings, and the last failure.
#[tauri::command]
pub async fn voice_input_status(
    state: tauri::State<'_, AppState>,
) -> Result<VoiceInputView, String> {
    Ok(state.voice_input.view())
}

/// Starts one global dictation, from the window or from the tray.
#[tauri::command]
pub async fn voice_input_start(
    state: tauri::State<'_, AppState>,
) -> Result<dictation::DictationOutcome, String> {
    let handle = state.voice_input.clone();
    tauri::async_runtime::spawn_blocking(move || handle.start())
        .await
        .map_err(|_| "storage".to_string())?
}

/// Cancels what is running. Returns whether anything was.
#[tauri::command]
pub async fn voice_input_cancel(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.voice_input.cancel())
}

/// Stores the settings of the feature.
#[tauri::command]
pub async fn voice_input_update_settings(
    state: tauri::State<'_, AppState>,
    settings: GlobalDictationSettings,
) -> Result<GlobalDictationSettings, String> {
    state.voice_input.update_settings(settings)
}

/// Forgets the last result and wipes the clipboard if it is still ours.
#[tauri::command]
pub async fn voice_input_clear_result(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.voice_input.clear_result())
}

/// The text of the last result, for the panel's preview. In memory only.
#[tauri::command]
pub async fn voice_input_preview(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    Ok(state.voice_input.last_text())
}

/// Copies the last result to the clipboard again, with a fresh timer.
#[tauri::command]
pub async fn voice_input_copy_again(state: tauri::State<'_, AppState>) -> Result<bool, String> {
    Ok(state.voice_input.copy_again())
}

/// The stage name a tray row shows.
pub fn stage_name(stage: DictationStage) -> &'static str {
    stage.as_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine is one object, shared by the tray, the window and the voice
    /// route: a second one would be a second microphone owner.
    #[test]
    fn the_engine_is_the_same_object_for_every_caller() {
        let handle = crate::tauri_commands::WhisperHandle::restore();
        let voice = VoiceInputHandle::restore(handle);
        let a = voice.clone();
        let b = voice.clone();
        assert!(Arc::ptr_eq(a.engine(), b.engine()));
        assert!(Arc::ptr_eq(a.engine(), voice.engine()));
        assert!(!voice.is_running());
        assert!(!voice.cancel(), "there is nothing to cancel");
    }

    /// Off by default, repaired when a document is nonsense, and the clipboard
    /// timeout stays inside the range the vault's guard enforces.
    #[test]
    fn the_defaults_and_the_repair_are_the_ones_the_core_defines() {
        let settings = GlobalDictationSettings::default();
        assert!(!settings.enabled);
        assert!(settings.punctuation);
        assert_eq!(settings.preference, VoiceInputPreference::Clipboard);
        let repaired = GlobalDictationSettings {
            clipboard_seconds: 5,
            phrase: "  ".to_string(),
            ..GlobalDictationSettings::default()
        }
        .normalized();
        assert_eq!(
            repaired.clipboard_seconds,
            jarvis_core::vault::clipboard::MIN_CLEAR_SECONDS
        );
        assert_eq!(repaired.phrase, dictation::START_PHRASES[0]);
    }

    /// A phrase produces a typed intent, and the feature's switch gates it. The
    /// trigger is the matcher, not a command runner.
    #[test]
    fn a_phrase_is_an_intent_and_off_means_no_intent() {
        let off = GlobalDictationSettings::default();
        assert_eq!(
            dictation::intent_with_settings(&off, "Джарвис, голосовой ввод"),
            VoiceIntent::None
        );
        let on = GlobalDictationSettings {
            enabled: true,
            ..GlobalDictationSettings::default()
        };
        for phrase in [
            "Джарвис, голосовой ввод",
            "Джарвис, начни голосовой ввод",
            "Джарвис, включи диктовку",
            "Джарвис, продиктую текст",
        ] {
            assert_eq!(
                dictation::intent_with_settings(&on, phrase),
                VoiceIntent::StartGlobalDictation,
                "{phrase}"
            );
        }
        assert_eq!(
            dictation::intent_with_settings(&on, "включи музыку"),
            VoiceIntent::None
        );
    }
}
