//! The sequence: intent, confirmation, microphone handover, recording,
//! transcription, punctuation, delivery, and the listener back again.
//!
//! # The one rule this module exists to enforce
//!
//! There is one microphone. The listener (Vosk, in the voice host) holds it, and
//! it must be *given up* before the dictation (Whisper, in this process) asks for
//! it — otherwise the second claim is refused and the person is told the
//! microphone is busy, which is exactly the failure this ordering prevents. The
//! host is a trait, so the order is a value a test can assert:
//!
//! ```text
//! capture target ─► speak confirmation ─► host.release() ─► dictate() ─► deliver
//!                                             │                            │
//!                                             └──────── always ───────────► host.restore()
//! ```
//!
//! The restore runs on every path, including a failed transcription and a
//! cancellation, because a listener that never comes back is a feature that broke
//! the assistant.
//!
//! # One request at a time
//!
//! A second request while one is running is refused with `busy`, whether it came
//! from the voice layer, from the tray, or from the window.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;

use super::error::DictationError;
use super::insertion::{deliver, ClipboardWriter, Delivery, ForegroundProbe, TextInserter};
use super::punctuation::apply_voice_punctuation;
use super::target::{TargetSnapshot, VoiceInputPreference};
use crate::vault::clipboard::clamp_clear_seconds;

/// Where a request has got to. The tray shows these names.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationStage {
    /// Nothing is running.
    Idle,
    /// The request is accepted and the focused element is being read.
    Preparing,
    /// The confirmation is being spoken.
    Confirming,
    /// The listener is giving the microphone up.
    Handover,
    /// The microphone is open and the person is speaking.
    Recording,
    /// The model is working on the audio.
    Transcribing,
    /// The text is being corrected and punctuated.
    Correcting,
    /// The text is being delivered.
    Inserting,
    /// The text was delivered.
    Delivered,
    /// The request failed, with a code.
    Failed,
    /// The request was cancelled before delivery.
    Cancelled,
}

impl DictationStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Preparing => "preparing",
            Self::Confirming => "confirming",
            Self::Handover => "handover",
            Self::Recording => "recording",
            Self::Transcribing => "transcribing",
            Self::Correcting => "correcting",
            Self::Inserting => "inserting",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether a request is in flight.
    pub fn is_running(&self) -> bool {
        !matches!(
            self,
            Self::Idle | Self::Delivered | Self::Failed | Self::Cancelled
        )
    }
}

/// The voice host: whoever holds the microphone between requests.
pub trait VoiceHost: Send + Sync {
    /// Stops the listener and gives the microphone up.
    ///
    /// Returns only when the microphone is free; the caller starts the dictation
    /// immediately afterwards.
    fn release_microphone(&self) -> Result<(), DictationError>;

    /// Starts the listener again.
    fn restore_listener(&self) -> Result<(), DictationError>;
}

/// The dictation itself: one recording, one transcription.
pub trait VoiceTranscriber: Send + Sync {
    /// Records and transcribes, and reports how long the audio was.
    fn transcribe(&self) -> Result<TranscribedText, DictationError>;
}

/// What a transcription produced, without the text being logged anywhere.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TranscribedText {
    pub text: String,
    pub audio_ms: u64,
}

/// The correction pass: the local spelling layer.
pub trait TextCorrector: Send + Sync {
    /// Corrects the text, or returns it unchanged.
    fn correct(&self, text: &str) -> String;
}

/// A corrector that changes nothing, for a platform or a configuration without
/// the spelling layer.
pub struct PassiveCorrector;

impl TextCorrector for PassiveCorrector {
    fn correct(&self, text: &str) -> String {
        text.to_string()
    }
}

/// What one request produced.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DictationOutcome {
    /// The stage the request ended in.
    pub stage: DictationStage,
    /// The length of the delivered text, never the text.
    pub characters: usize,
    /// How it was delivered, when it was.
    pub method: Option<super::insertion::DeliveryMethod>,
    /// The rule behind a refusal or a fallback.
    pub rule: Option<&'static str>,
    /// A content-free code when the request failed.
    pub error_code: Option<&'static str>,
    /// Audio length that was transcribed, in milliseconds.
    pub audio_ms: u64,
}

impl DictationOutcome {
    fn delivered(delivery: Delivery, audio_ms: u64) -> Self {
        Self {
            stage: DictationStage::Delivered,
            characters: delivery.characters,
            method: Some(delivery.method),
            rule: delivery.rule,
            error_code: None,
            audio_ms,
        }
    }
}

/// One request, as the caller describes it.
pub struct DictationRequest {
    /// Whether the local spelling layer runs on the result.
    pub autocorrect: bool,
    /// Whether voice punctuation runs on the result.
    pub punctuation: bool,
    pub preference: VoiceInputPreference,
    /// Whether the confirmation is spoken.
    pub speak_confirmation: bool,
}

/// Everything one request needs, so the engine itself holds no platform code.
pub struct EngineDeps<'a> {
    pub host: &'a dyn VoiceHost,
    pub transcriber: &'a dyn VoiceTranscriber,
    pub corrector: &'a dyn TextCorrector,
    pub probe: &'a dyn ForegroundProbe,
    pub inserter: &'a dyn TextInserter,
    pub clipboard: &'a dyn ClipboardWriter,
    /// Speaks the confirmation. A no-op is a valid implementation.
    pub speak: &'a dyn Fn(&str),
}

/// The engine: one microphone, one request, one text.
pub struct DictationEngine {
    stage: Mutex<DictationStage>,
    cancel: Arc<AtomicBool>,
    /// The transcript of the last delivered request, in memory only.
    ///
    /// It exists so the panel can offer a preview and "insert again"; it is
    /// cleared by the next request, by `forget`, and by the exit. Nothing writes
    /// it to disk, to a log, or to the window's storage.
    last: Mutex<Option<String>>,
    /// Length of the last delivered text, for the status.
    last_characters: Mutex<usize>,
}

impl Default for DictationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl DictationEngine {
    pub fn new() -> Self {
        Self {
            stage: Mutex::new(DictationStage::Idle),
            cancel: Arc::new(AtomicBool::new(false)),
            last: Mutex::new(None),
            last_characters: Mutex::new(0),
        }
    }

    pub fn stage(&self) -> DictationStage {
        *self.stage.lock()
    }

    /// Whether a request is running, so a second one is refused rather than
    /// queued behind the microphone.
    pub fn is_running(&self) -> bool {
        self.stage().is_running()
    }

    /// The handler a cancel button, Escape, or the tray calls.
    ///
    /// The flag is read at the safe points of the route, and the request never
    /// delivers anything after it is set. It is bounded: nothing waits for the
    /// transcription to notice.
    pub fn cancel(&self) -> bool {
        if !self.is_running() {
            return false;
        }
        self.cancel.store(true, Ordering::SeqCst);
        true
    }

    /// The text of the last request, for the panel's preview. In memory only.
    pub fn last_text(&self) -> Option<String> {
        self.last.lock().clone()
    }

    pub fn last_characters(&self) -> usize {
        *self.last_characters.lock()
    }

    /// Forgets the last text.
    pub fn forget(&self) {
        *self.last.lock() = None;
        *self.last_characters.lock() = 0;
    }

    /// Runs one request to the end, and returns what it produced.
    ///
    /// Every failure is a value. The listener is restored on every path, and a
    /// second request while one is running is refused before anything happens.
    pub fn run(
        self: &Arc<Self>,
        request: DictationRequest,
        deps: EngineDeps<'_>,
    ) -> Result<DictationOutcome, DictationError> {
        {
            let mut stage = self.stage.lock();
            if stage.is_running() {
                return Err(DictationError::Busy);
            }
            *stage = DictationStage::Preparing;
            self.cancel.store(false, Ordering::SeqCst);
            *self.last.lock() = None;
            *self.last_characters.lock() = 0;
        }

        let result = self.run_inner(&request, &deps);
        let audio_ms = match &result {
            Ok(outcome) => outcome.audio_ms,
            Err(_) => 0,
        };
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                *self.stage.lock() = match error {
                    DictationError::Cancelled => DictationStage::Cancelled,
                    _ => DictationStage::Failed,
                };
                log::warn!(
                    "dictation: request failed (stage={} error_code={})",
                    self.stage().as_str(),
                    error.code()
                );
                *self.stage.lock() = DictationStage::Idle;
                return Err(error);
            }
        };
        log::info!(
            "dictation: delivered (stage=delivered method={} characters={} audio_ms={})",
            outcome
                .method
                .map(|method| match method {
                    super::insertion::DeliveryMethod::UiAutomation => "ui_automation",
                    super::insertion::DeliveryMethod::Clipboard => "clipboard",
                })
                .unwrap_or("none"),
            outcome.characters,
            audio_ms
        );
        *self.stage.lock() = DictationStage::Idle;
        Ok(outcome)
    }

    fn run_inner(
        &self,
        request: &DictationRequest,
        deps: &EngineDeps<'_>,
    ) -> Result<DictationOutcome, DictationError> {
        // 1. the focused element, before anything is spoken: the person may move
        //    the window while the confirmation is playing, and this is what the
        //    text was meant for.
        let target = deps.probe.focused_target()?;
        self.check_cancelled()?;

        // 2. the confirmation, so the person knows the assistant heard them.
        *self.stage.lock() = DictationStage::Confirming;
        if request.speak_confirmation {
            (deps.speak)(super::confirmation("ru"));
        }
        self.check_cancelled()?;

        // 3. the microphone changes hands. The listener must be gone before the
        //    dictation asks for the device, and it comes back on every path.
        *self.stage.lock() = DictationStage::Handover;
        deps.host.release_microphone()?;
        let guard = ListenerGuard {
            host: deps.host,
            armed: true,
        };
        self.check_cancelled()?;

        // 4. the recording and the transcription, which is the existing session.
        *self.stage.lock() = DictationStage::Recording;
        let transcribed = deps.transcriber.transcribe()?;
        *self.stage.lock() = DictationStage::Transcribing;
        if transcribed.text.trim().is_empty() {
            drop(guard);
            return Err(DictationError::EmptyRecording);
        }
        self.check_cancelled()?;

        // 5. the local passes: spelling, then the spoken marks.
        *self.stage.lock() = DictationStage::Correcting;
        let mut text = if request.autocorrect {
            deps.corrector.correct(&transcribed.text)
        } else {
            transcribed.text.clone()
        };
        if request.punctuation {
            let (punctuated, report) = apply_voice_punctuation(&text);
            if report.changed {
                log::info!(
                    "dictation: voice punctuation applied (marks={} characters={})",
                    report.marks,
                    punctuated.chars().count()
                );
            }
            text = punctuated;
        }
        self.check_cancelled()?;

        // 6. delivery, through the gate.
        *self.stage.lock() = DictationStage::Inserting;
        let delivery = deliver(
            &target,
            request.preference,
            &text,
            deps.probe,
            deps.inserter,
            deps.clipboard,
        )?;
        *self.last.lock() = Some(text);
        *self.last_characters.lock() = delivery.characters;
        drop(guard);
        Ok(DictationOutcome::delivered(delivery, transcribed.audio_ms))
    }

    fn check_cancelled(&self) -> Result<(), DictationError> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(DictationError::Cancelled);
        }
        Ok(())
    }
}

/// Gives the microphone back to the listener, whatever happened.
///
/// A `Drop` rather than a call at the end, so a returned error, an early exit,
/// and a cancelled request all restore the listener. A restore that fails is
/// logged and reported by the request that owns it: the alternative — leaving the
/// assistant deaf — is worse than a failure the person can see.
struct ListenerGuard<'a> {
    host: &'a dyn VoiceHost,
    armed: bool,
}

impl Drop for ListenerGuard<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        match self.host.restore_listener() {
            Ok(()) => log::info!("dictation: the listener has the microphone again"),
            Err(error) => log::warn!(
                "dictation: the listener could not take the microphone back (error_code={})",
                error.code()
            ),
        }
    }
}

/// A settings value, repaired.
pub fn normalized_clipboard_seconds(seconds: u64) -> u64 {
    clamp_clear_seconds(seconds)
}

/// The outcome of the last request, for the tray, without the text.
pub fn status_of(engine: &DictationEngine) -> DictationStatusView {
    DictationStatusView {
        stage: engine.stage(),
        characters: engine.last_characters(),
        has_text: engine.last_text().is_some(),
    }
}

/// What the tray and the panel show.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
pub struct DictationStatusView {
    pub stage: DictationStage,
    pub characters: usize,
    /// Whether a text is being held in memory, never the text itself.
    pub has_text: bool,
}

/// A target snapshot read before the confirmation, kept for the tests.
pub fn snapshot_for_test(target: &TargetSnapshot) -> TargetSnapshot {
    target.clone()
}
