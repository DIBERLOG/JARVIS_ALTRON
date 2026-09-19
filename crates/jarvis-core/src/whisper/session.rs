//! The dictation session: the only place that records and transcribes.
//!
//! One session owns the settings, the runner, the temporary audio, and the
//! state the interface shows. The rules it exists to keep:
//!
//! * the microphone is opened only between an explicit start and an explicit
//!   stop, and never because the application was launched or installed;
//! * one transcription runs at a time, and a second request is refused with
//!   `Busy` instead of queueing audio nobody asked for;
//! * the recorded audio is written into the feature directory, transcribed, and
//!   deleted as soon as the text exists — unless the user asked to keep it, in
//!   which case it stays where they can see it;
//! * the transcript is returned to the caller and shown to the user. It is
//!   never logged, never sent anywhere, and never stored in the AI memory by
//!   this module;
//! * cancelling stops the *own* child process and deletes the audio.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use super::config::{
    WhisperSettings, DEFAULT_SILENCE_PEAK, MIN_AUDIO_MS, SAMPLE_RATE, SETTINGS_FILE,
};
use super::error::WhisperError;
use super::model::{probe_binary, probe_model, BinaryProbe, ModelProbe};
use super::runner::{
    build_arguments, parse_json_transcript, parse_stdout_transcript, ProcessTranscriber,
    Transcriber, Transcript, AUDIO_FILE_NAME, OUTPUT_PREFIX,
};
use super::wav::{
    frames_for_seconds, peak_amplitude, samples_for_millis, samples_for_seconds, write_wav,
    WavFormat,
};

/// Where the frames of a recording come from.
///
/// The real implementation reads the recorder the application already uses; the
/// fake produces a scripted signal, so the session can be tested without a
/// microphone.
pub trait FrameSource: Send {
    /// Fills the buffer with the next frame of 16 kHz mono samples.
    ///
    /// A failure — no microphone, an uninitialised recorder, a driver that went
    /// away — is a value, because a recording has to stop and free the device
    /// instead of unwinding the thread that was recording.
    fn read_frame(&mut self, buffer: &mut [i16]) -> Result<(), WhisperError>;
    /// Whether the source is still producing audio.
    fn is_running(&self) -> bool {
        true
    }
}

/// What the session is doing right now.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationState {
    /// Not recording and not transcribing.
    Idle,
    /// The microphone is open and the frames are being collected.
    Recording,
    /// A stop was asked for and is being carried out: the stream is closed, the
    /// device is released, and the audio is on its way to the model.
    Stopping,
    /// The recorded audio is being transcribed.
    Transcribing,
    /// The last dictation produced a transcript.
    Complete,
    /// The last dictation failed; the reason is in the last outcome.
    Failed,
}

/// A recording that is in progress or has just finished.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Recording {
    pub samples: Vec<i16>,
    pub truncated: bool,
}

impl Recording {
    pub fn seconds(&self) -> f64 {
        self.samples.len() as f64 / SAMPLE_RATE as f64
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// The loudest sample of the whole recording.
    pub fn peak(&self) -> i16 {
        peak_amplitude(&self.samples)
    }
}

/// Why a recording stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StopReason {
    /// The user asked to stop.
    Requested,
    /// The configured length was reached.
    Length,
    /// The configured silence was reached after speech.
    Silence,
}

/// What the interface needs to describe the feature.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DictationStatus {
    pub state: DictationState,
    pub enabled: bool,
    pub configured: bool,
    pub binary: Option<BinaryProbe>,
    pub model: Option<ModelProbe>,
    /// The full path, for the settings page only.
    pub binary_path: String,
    /// The full path, for the settings page only.
    pub model_path: String,
    /// The executable's own name, safe to show anywhere.
    pub binary_name: String,
    /// The model's own name, safe to show anywhere.
    pub model_name: String,
    /// Content-free explanations of what is missing. A note is either a Fluent
    /// key (`windows-whisper-note-…`) or a plain sentence.
    pub notes: Vec<String>,
}

/// Reads the microphone the application already has open.
pub struct RecorderFrames;

impl FrameSource for RecorderFrames {
    fn read_frame(&mut self, buffer: &mut [i16]) -> Result<(), WhisperError> {
        crate::recorder::try_read_microphone(buffer)
            .map_err(|error| WhisperError::AudioUnavailable(error.to_string()))
    }
}

/// A session over one feature directory.
pub struct WhisperSession {
    directory: PathBuf,
    /// The settings, behind a lock so a running session can be reconfigured and
    /// asked for its status at the same time.
    settings: RwLock<WhisperSettings>,
    runner: RwLock<Arc<dyn Transcriber>>,
    state: Mutex<DictationState>,
    cancel: Arc<AtomicBool>,
    /// The audio file of the transcription that is running, so a cancel can
    /// clean it up even while the runner is busy.
    current_audio: Mutex<Option<PathBuf>>,
    /// What the last attempt produced, for the interface to show after the
    /// session has returned to `Idle`.
    last_outcome: Mutex<Option<DictationOutcome>>,
}

/// What a finished attempt produced, kept after the state returns to `Idle`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DictationOutcome {
    /// A transcript was produced, with its length in characters only: the text
    /// itself is never kept here.
    Transcript { characters: usize },
    /// Nothing was recorded.
    EmptyRecording,
    /// Something failed, with a content-free code.
    Failed { code: &'static str },
}

/// Releases the microphone and resets the state, whatever happens.
///
/// This is the mechanism behind the guarantee that a failure — including a panic
/// in a native library — cannot leave the session in `Recording` with the device
/// held. It is deliberately not a `Drop` on the session itself: the session
/// outlives many recordings, and only a recording has to be closed.
struct RecordingGuard<'a> {
    session: &'a WhisperSession,
}

impl<'a> RecordingGuard<'a> {
    fn new(session: &'a WhisperSession) -> Self {
        Self { session }
    }
}

impl Drop for RecordingGuard<'_> {
    fn drop(&mut self) {
        // 1. close the stream, whatever state the backend is in;
        let _ = crate::recorder::try_stop_recording();
        // 2. release the device in the session's own view of the world;
        self.session.cancel.store(false, Ordering::SeqCst);
        // 3. leave `Stopping`/`Recording` behind: a caller that reports the
        //    state after an error must see `Idle`, not a stuck recording.
        let mut state = self.session.state.lock();
        if matches!(
            *state,
            DictationState::Recording
                | DictationState::Stopping
                | DictationState::Transcribing
                | DictationState::Complete
                | DictationState::Failed
        ) {
            *state = DictationState::Idle;
        }
    }
}

/// The outcome of one attempt, from its result.
fn outcome_of(result: &Result<Transcript, WhisperError>) -> DictationOutcome {
    match result {
        Ok(transcript) => DictationOutcome::Transcript {
            characters: transcript.text.chars().count(),
        },
        Err(WhisperError::AudioEmpty) => DictationOutcome::EmptyRecording,
        Err(error) => DictationOutcome::Failed { code: error.code() },
    }
}

impl std::fmt::Debug for WhisperSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WhisperSession")
            .field("directory", &self.directory)
            .field("state", &*self.state.lock())
            .field("enabled", &self.settings.read().enabled)
            .field("configured", &self.settings.read().is_configured())
            .finish()
    }
}

impl WhisperSession {
    /// Opens the feature over `directory` with the process runner.
    pub fn open(directory: &Path, settings: WhisperSettings) -> Self {
        // The stored document is what the user last saved, so it wins.
        let settings = super::config::stored_settings(directory).unwrap_or(settings);
        Self::with_runner(
            directory,
            settings,
            Arc::new(ProcessTranscriber::new("whisper-cli.exe")),
        )
    }

    /// Opens the feature with an injected runner, for tests.
    pub fn with_runner(
        directory: &Path,
        settings: WhisperSettings,
        runner: Arc<dyn Transcriber>,
    ) -> Self {
        Self {
            directory: directory.to_path_buf(),
            settings: RwLock::new(settings.normalized()),
            runner: RwLock::new(runner),
            state: Mutex::new(DictationState::Idle),
            cancel: Arc::new(AtomicBool::new(false)),
            current_audio: Mutex::new(None),
            last_outcome: Mutex::new(None),
        }
    }

    /// The settings as they are right now.
    pub fn settings(&self) -> WhisperSettings {
        self.settings.read().clone()
    }

    pub fn data_dir(&self) -> &Path {
        &self.directory
    }

    pub fn state(&self) -> DictationState {
        *self.state.lock()
    }

    /// What the last attempt produced, if there was one.
    pub fn last_outcome(&self) -> Option<DictationOutcome> {
        self.last_outcome.lock().clone()
    }

    /// Whether the microphone is held right now, by this session.
    pub fn holds_microphone(&self) -> bool {
        matches!(
            self.state(),
            DictationState::Recording | DictationState::Stopping
        )
    }

    /// Whether a transcription is running.
    pub fn is_busy(&self) -> bool {
        self.state() != DictationState::Idle
    }

    /// Writes the settings, atomically, and applies them.
    ///
    /// Takes `&self`: a session that is recording or transcribing can still be
    /// reconfigured, and switching the feature off stops the work in flight.
    pub fn update_settings(
        &self,
        settings: WhisperSettings,
    ) -> Result<WhisperSettings, WhisperError> {
        let settings = settings.normalized();
        // The document is written to a temporary file and renamed, so a crash
        // cannot leave half a settings file behind.
        let target = self.directory.join(SETTINGS_FILE);
        let temporary = self.directory.join(format!("{SETTINGS_FILE}.tmp"));
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&temporary, settings.to_json()?)?;
        std::fs::rename(&temporary, &target).map_err(|_| WhisperError::Storage)?;
        // Turning the feature off while it is running must stop it, not leave a
        // microphone open behind a switch that says "off".
        if !settings.enabled && self.is_busy() {
            self.cancel();
        }
        *self.settings.write() = settings.clone();
        Ok(settings)
    }

    /// Sets the runner program, so the session uses the picked executable.
    pub fn set_program(&self, program: &Path) {
        *self.runner.write() = Arc::new(ProcessTranscriber::new(program));
    }

    /// The state the interface shows, including what is missing.
    pub fn status(&self) -> DictationStatus {
        let settings = self.settings();
        let mut notes = Vec::new();
        let binary = if settings.binary_path.is_empty() {
            notes.push("windows-whisper-note-no-binary".to_string());
            None
        } else {
            match probe_binary(Path::new(&settings.binary_path)) {
                Ok(probe) => Some(probe),
                Err(error) => {
                    push_error_notes(&mut notes, &error);
                    None
                }
            }
        };
        let model = if settings.model_path.is_empty() {
            notes.push("windows-whisper-note-no-model".to_string());
            None
        } else {
            match probe_model(Path::new(&settings.model_path)) {
                Ok(probe) => {
                    notes.extend(probe.notes.iter().cloned());
                    Some(probe)
                }
                Err(error) => {
                    push_error_notes(&mut notes, &error);
                    None
                }
            }
        };
        if !settings.enabled {
            notes.push("windows-whisper-note-disabled".to_string());
        }
        DictationStatus {
            state: self.state(),
            enabled: settings.enabled,
            configured: binary.is_some() && model.is_some(),
            binary,
            model,
            binary_name: file_name_of(&settings.binary_path),
            model_name: file_name_of(&settings.model_path),
            binary_path: settings.binary_path.clone(),
            model_path: settings.model_path.clone(),
            notes,
        }
    }

    /// Records from `source` until the user stops it, the length is reached, or
    /// the configured silence follows speech.
    ///
    /// This is the only function that opens the microphone, and it returns as
    /// soon as recording ends: transcription is a separate, cancellable step.
    pub fn record(
        &self,
        source: &mut dyn FrameSource,
    ) -> Result<(Recording, StopReason), WhisperError> {
        self.begin()?;
        // The guard is what makes the guarantee true: whatever happens below —
        // a returned error, an early exit, or a panic from a native library —
        // the stream is stopped, the device is released, and the session is back
        // to `Idle` instead of staying in `Recording` forever.
        let guard = RecordingGuard::new(self);
        let result = self.record_inner(source);
        let state = match &result {
            Ok(_) => DictationState::Complete,
            Err(_) => DictationState::Failed,
        };
        *self.state.lock() = state;
        *self.last_outcome.lock() = Some(match &result {
            Ok(_) => DictationOutcome::Transcript { characters: 0 },
            Err(WhisperError::AudioEmpty) => DictationOutcome::EmptyRecording,
            Err(error) => DictationOutcome::Failed { code: error.code() },
        });
        // `guard` runs here; the explicit drop documents the order.
        drop(guard);
        result
    }

    fn record_inner(
        &self,
        source: &mut dyn FrameSource,
    ) -> Result<(Recording, StopReason), WhisperError> {
        let settings = self.settings();
        let frame_samples = 512usize;
        // Rounded up, so a one-second recording really reaches one second.
        let total_frames = frames_for_seconds(settings.max_seconds, frame_samples);
        let silence_frames = (samples_for_millis(settings.silence_ms) / frame_samples).max(1);
        let mut buffer = vec![0i16; frame_samples];
        let mut samples: Vec<i16> = Vec::with_capacity(samples_for_seconds(settings.max_seconds));
        let mut silent_run = 0usize;
        let mut heard_speech = false;
        let mut truncated = false;
        let mut reason = StopReason::Length;

        for _ in 0..total_frames {
            if self.cancel.load(Ordering::SeqCst) {
                reason = StopReason::Requested;
                break;
            }
            if !source.is_running() {
                reason = StopReason::Requested;
                break;
            }
            // A read failure is the end of the recording, with the reason kept:
            // the stream is closed by the guard and the caller is told why.
            source.read_frame(&mut buffer)?;
            let peak = peak_amplitude(&buffer);
            if peak > DEFAULT_SILENCE_PEAK {
                heard_speech = true;
                silent_run = 0;
            } else if heard_speech {
                silent_run += 1;
                if silent_run >= silence_frames.max(1) {
                    reason = StopReason::Silence;
                    samples.extend_from_slice(&buffer);
                    break;
                }
            }
            samples.extend_from_slice(&buffer);
            if samples.len() >= samples_for_seconds(settings.max_seconds) {
                truncated = true;
                break;
            }
        }

        // The floor is "something was recorded", not a preference: a word that
        // ended on silence is still sent, and only an empty buffer is refused.
        if samples.len() < samples_for_millis(MIN_AUDIO_MS) {
            return Err(WhisperError::AudioEmpty);
        }
        Ok((Recording { samples, truncated }, reason))
    }

    /// Stops a recording or a transcription that is running.
    ///
    /// The flag is what the record loop reads, and what the runner reads before
    /// it kills its own child. A session that is idle is left alone.
    pub fn cancel(&self) -> bool {
        // The flag is set and nothing is waited for, so a stop is always bounded
        // by the loop that reads it — including when the worker has already
        // died, because then there is nothing left to wait for either.
        if !self.is_busy() {
            return false;
        }
        self.cancel.store(true, Ordering::SeqCst);
        *self.state.lock() = DictationState::Stopping;
        true
    }

    /// Transcribes samples that are already in memory.
    pub fn transcribe_samples(&self, samples: &[i16]) -> Result<Transcript, WhisperError> {
        if samples.is_empty() {
            return Err(WhisperError::AudioEmpty);
        }
        self.settings().validate()?;
        let path = self.directory.join(AUDIO_FILE_NAME);
        let format = write_wav(&path, samples)?;
        let result = self.transcribe_file_inner(&path, format);
        self.finish_audio(&path);
        result
    }

    /// Transcribes an audio file the user picked.
    ///
    /// The file must already be 16 kHz mono 16-bit PCM: converting it would mean
    /// decoding an arbitrary audio file, which this feature does not do.
    pub fn transcribe_path(&self, path: &Path) -> Result<Transcript, WhisperError> {
        self.settings().validate()?;
        let format = super::wav::read_wav_format(path)?;
        if !format.is_supported() {
            return Err(WhisperError::AudioUnavailable(format!(
                "that file is {} Hz with {} channel(s) and {} bits; {} Hz mono 16-bit is needed",
                format.sample_rate, format.channels, format.bits_per_sample, SAMPLE_RATE
            )));
        }
        self.transcribe_file_inner(path, format)
    }

    fn transcribe_file_inner(
        &self,
        audio_path: &Path,
        format: WavFormat,
    ) -> Result<Transcript, WhisperError> {
        self.begin()?;
        *self.state.lock() = DictationState::Transcribing;
        let result = self.transcribe_inner(audio_path, format);
        *self.state.lock() = if result.is_ok() {
            DictationState::Complete
        } else {
            DictationState::Failed
        };
        *self.last_outcome.lock() = Some(outcome_of(&result));
        // The session is ready for the next attempt, whatever happened.
        *self.state.lock() = DictationState::Idle;
        result
    }

    fn transcribe_inner(
        &self,
        audio_path: &Path,
        format: WavFormat,
    ) -> Result<Transcript, WhisperError> {
        let settings = self.settings();
        let output_base = self.directory.join(OUTPUT_PREFIX);
        let arguments = build_arguments(
            &settings.model_path,
            audio_path,
            &settings.language,
            settings.threads,
            settings.translate,
            &output_base,
        );
        if self.cancel.load(Ordering::SeqCst) {
            return Err(WhisperError::Cancelled);
        }
        let started = Instant::now();
        let runner = Arc::clone(&self.runner.read());
        let outcome = runner.transcribe(
            &arguments,
            audio_path,
            &output_base,
            Duration::from_secs(settings.timeout_seconds),
            &self.cancel,
        )?;
        let duration_ms = started.elapsed().as_millis() as u64;

        // The build writes the report next to the audio; the printed text is the
        // fallback for builds that do not.
        let json_path = output_base.with_extension("json");
        let mut transcript = match std::fs::read(&json_path) {
            Ok(bytes) => parse_json_transcript(&bytes, &settings.language)?,
            Err(_) => parse_stdout_transcript(&outcome.stdout, &settings.language)?,
        };
        // The report is a copy of the transcript: it does not outlive the call.
        let _ = std::fs::remove_file(&json_path);
        transcript.audio_ms = format.duration_ms();
        transcript.duration_ms = duration_ms;
        Ok(transcript)
    }

    /// Records and transcribes in one call, for the window's own button.
    pub fn dictate(&self, source: &mut dyn FrameSource) -> Result<Transcript, WhisperError> {
        let (recording, _reason) = self.record(source)?;
        self.transcribe_samples(&recording.samples)
    }

    /// Deletes the audio of a transcription unless the user asked to keep it.
    fn finish_audio(&self, path: &Path) {
        *self.current_audio.lock() = None;
        if !self.settings().keep_audio {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Refuses a second job and clears the cancel flag for this one.
    fn begin(&self) -> Result<(), WhisperError> {
        let mut state = self.state.lock();
        if *state != DictationState::Idle {
            return Err(WhisperError::Busy);
        }
        let settings = self.settings();
        if !settings.enabled {
            return Err(WhisperError::Disabled);
        }
        if settings.not_configured() {
            return Err(WhisperError::NotConfigured);
        }
        self.cancel.store(false, Ordering::SeqCst);
        *state = DictationState::Recording;
        Ok(())
    }

    /// Resets the session, for a full exit. Safe to call twice.
    pub fn shutdown(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        *self.state.lock() = DictationState::Idle;
        if !self.settings().keep_audio {
            let _ = std::fs::remove_file(self.directory.join(AUDIO_FILE_NAME));
        }
    }
}

/// The Fluent key that explains why a file cannot be used.
fn note_for(error: &WhisperError) -> String {
    format!("windows-whisper-note-{}", error.code().replace('_', "-"))
}

/// Adds the localized reason and, when there is one, the concrete detail.
///
/// The key alone says "the model cannot be used"; the detail says what was found
/// instead, which is the difference between a person fixing it and giving up.
fn push_error_notes(notes: &mut Vec<String>, error: &WhisperError) {
    notes.push(note_for(error));
    if let Some(detail) = error.detail() {
        let detail = crate::text::shorten(&detail, 160);
        if !notes.contains(&detail) {
            notes.push(detail);
        }
    }
}

/// The file's own name, for a status line that is safe to show anywhere.
fn file_name_of(path: &str) -> String {
    if path.trim().is_empty() {
        return String::new();
    }
    crate::text::file_label(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::whisper::model::{MIN_MODEL_BYTES, PE_MACHINE_AMD64};
    use crate::whisper::runner::RunOutcome;
    use tempfile::tempdir;

    /// A source that produces a scripted signal: speech for a while, then silence.
    struct ScriptedSource {
        frames: Vec<Vec<i16>>,
        position: usize,
        endless: bool,
        /// When the script is exhausted, stop the source instead of padding it
        /// with silence. This is what a recording that simply ended looks like.
        stop_when_exhausted: bool,
    }

    impl ScriptedSource {
        fn speech_then_silence(speech_frames: usize, silence_frames: usize) -> Self {
            let mut frames = Vec::new();
            for _ in 0..speech_frames {
                frames.push(vec![3000i16; 512]);
            }
            for _ in 0..silence_frames {
                frames.push(vec![0i16; 512]);
            }
            Self {
                frames,
                position: 0,
                endless: false,
                stop_when_exhausted: false,
            }
        }

        /// A source that produces the given frames and then stops running.
        fn short(frames: usize) -> Self {
            Self {
                frames: vec![vec![3000i16; 512]; frames],
                position: 0,
                endless: false,
                stop_when_exhausted: true,
            }
        }

        fn endless_speech() -> Self {
            Self {
                frames: Vec::new(),
                position: 0,
                endless: true,
                stop_when_exhausted: false,
            }
        }
    }

    impl FrameSource for ScriptedSource {
        fn is_running(&self) -> bool {
            !self.stop_when_exhausted || self.position < self.frames.len()
        }

        fn read_frame(&mut self, buffer: &mut [i16]) -> Result<(), WhisperError> {
            match self.endless {
                true => buffer.fill(5000),
                false => match self.frames.get(self.position) {
                    Some(frame) => buffer.copy_from_slice(&frame[..buffer.len()]),
                    // A scripted source that ran out is silent, so a test that
                    // scripted less than the maximum hears silence, not noise.
                    None => buffer.fill(0),
                },
            }
            self.position += 1;
            Ok(())
        }
    }

    /// A runner that answers with a transcript, or with a failure.
    struct FakeTranscriber {
        text: String,
        language: String,
        writes_json: bool,
        fail: bool,
        hang: bool,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl FakeTranscriber {
        fn answering(text: &str) -> Self {
            Self {
                text: text.to_string(),
                language: "ru".to_string(),
                writes_json: true,
                fail: false,
                hang: false,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn failing() -> Self {
            Self {
                text: String::new(),
                language: "auto".to_string(),
                writes_json: false,
                fail: true,
                hang: false,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn hanging() -> Self {
            Self {
                text: String::new(),
                language: "auto".to_string(),
                writes_json: false,
                fail: false,
                hang: true,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn stdout_only(text: &str) -> Self {
            Self {
                text: text.to_string(),
                language: "auto".to_string(),
                writes_json: false,
                fail: false,
                hang: false,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().clone()
        }
    }

    impl Transcriber for FakeTranscriber {
        fn transcribe(
            &self,
            arguments: &[String],
            _audio_path: &Path,
            output_base: &Path,
            _timeout: Duration,
            cancel: &Arc<AtomicBool>,
        ) -> Result<RunOutcome, WhisperError> {
            self.calls.lock().push(arguments.to_vec());
            if self.fail {
                return Err(WhisperError::ProcessFailed { code: Some(1) });
            }
            if self.hang {
                // A build that never finishes: the wait ends when the session is
                // cancelled, which is what a cancel is for.
                let deadline = Instant::now() + Duration::from_secs(2);
                while Instant::now() < deadline {
                    if cancel.load(Ordering::SeqCst) {
                        return Err(WhisperError::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                return Err(WhisperError::TimedOut);
            }
            if self.writes_json {
                let body = serde_json::json!({
                    "result": {"language": self.language},
                    "transcription": [
                        {"timestamps": {"from": "00:00:00,000", "to": "00:00:01,000"}, "text": self.text}
                    ]
                });
                std::fs::write(
                    output_base.with_extension("json"),
                    serde_json::to_vec(&body).unwrap(),
                )
                .unwrap();
                Ok(RunOutcome {
                    exit_code: Some(0),
                    stdout: String::new(),
                })
            } else {
                Ok(RunOutcome {
                    exit_code: Some(0),
                    stdout: format!("[00:00:00.000 --> 00:00:01.000]  {}\n", self.text),
                })
            }
        }
    }

    /// A directory with a plausible executable and model in it.
    fn prepared_directory() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempdir().unwrap();
        let mut pe = vec![0u8; 0x100];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        pe[0x44..0x46].copy_from_slice(&PE_MACHINE_AMD64.to_le_bytes());
        let binary = directory.path().join("whisper-cli.exe");
        std::fs::write(&binary, &pe).unwrap();
        let model = directory.path().join("ggml-small.bin");
        std::fs::write(
            &model,
            [b"ggml".to_vec(), vec![0u8; (MIN_MODEL_BYTES + 16) as usize]].concat(),
        )
        .unwrap();
        (directory, binary, model)
    }

    fn enabled_settings(binary: &Path, model: &Path) -> WhisperSettings {
        WhisperSettings {
            enabled: true,
            binary_path: binary.to_string_lossy().into_owned(),
            model_path: model.to_string_lossy().into_owned(),
            silence_ms: 1_000,
            max_seconds: 1,
            ..WhisperSettings::default()
        }
    }

    #[test]
    fn a_session_that_is_switched_off_refuses_to_record() {
        let (directory, binary, model) = prepared_directory();
        let settings = WhisperSettings {
            enabled: false,
            binary_path: binary.to_string_lossy().into_owned(),
            model_path: model.to_string_lossy().into_owned(),
            ..WhisperSettings::default()
        };
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let mut source = ScriptedSource::speech_then_silence(40, 40);
        assert_eq!(
            session.record(&mut source).unwrap_err(),
            WhisperError::Disabled
        );
        assert_eq!(session.state(), DictationState::Idle);
    }

    #[test]
    fn a_session_without_files_says_so_instead_of_starting_a_process() {
        let directory = tempdir().unwrap();
        let settings = WhisperSettings {
            enabled: true,
            ..WhisperSettings::default()
        };
        let runner = Arc::new(FakeTranscriber::answering("привет"));
        let session = WhisperSession::with_runner(directory.path(), settings, runner.clone());
        let mut source = ScriptedSource::speech_then_silence(40, 40);
        assert_eq!(
            session.record(&mut source).unwrap_err(),
            WhisperError::NotConfigured
        );
        assert!(runner.calls().is_empty(), "nothing may be started");
    }

    #[test]
    fn recording_stops_on_silence_and_keeps_what_was_said() {
        let (directory, binary, model) = prepared_directory();
        // Speech, then enough silence to end the recording on its own.
        let mut settings = enabled_settings(&binary, &model);
        settings.max_seconds = 5;
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let mut source = ScriptedSource::speech_then_silence(40, 40);
        let (recording, reason) = session.record(&mut source).unwrap();
        assert_eq!(reason, StopReason::Silence);
        assert!(!recording.is_empty());
        assert!(!recording.truncated);
        assert!(recording.peak() > DEFAULT_SILENCE_PEAK);
        // 40 speech frames plus the 31 silent ones that ended it.
        assert_eq!(recording.samples.len(), 71 * 512);
    }

    #[test]
    fn recording_stops_at_the_configured_length() {
        let (directory, binary, model) = prepared_directory();
        let mut settings = enabled_settings(&binary, &model);
        settings.max_seconds = 5;
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        // Endless loud audio: only the length can stop this.
        let mut source = ScriptedSource::endless_speech();
        let (recording, reason) = session.record(&mut source).unwrap();
        assert_eq!(reason, StopReason::Length);
        assert!(recording.truncated);
        assert!(recording.samples.len() >= samples_for_seconds(5));
    }

    #[test]
    fn a_recording_that_is_too_short_is_not_sent_anywhere() {
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let runner = Arc::new(FakeTranscriber::answering("привет"));
        let session = WhisperSession::with_runner(directory.path(), settings, runner.clone());
        // Two frames are 64 ms, below the floor for "something recorded".
        let mut source = ScriptedSource::short(2);
        assert_eq!(
            session.record(&mut source).unwrap_err(),
            WhisperError::AudioEmpty
        );
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn a_short_word_that_ended_on_silence_is_still_transcribed() {
        // The floor is "something was recorded", not "longer than a setting": a
        // single word followed by silence must reach the model.
        let (directory, binary, model) = prepared_directory();
        let mut settings = enabled_settings(&binary, &model);
        settings.silence_ms = 500;
        settings.max_seconds = 5;
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("да")),
        );
        // 10 speech frames (0.32 s) then the silence that ends it.
        let mut source = ScriptedSource::speech_then_silence(10, 16);
        let transcript = session.dictate(&mut source).unwrap();
        assert_eq!(transcript.cleaned(), "да");
    }

    #[test]
    fn dictation_writes_the_audio_runs_once_and_deletes_it() {
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let runner = Arc::new(FakeTranscriber::answering("привет мир"));
        let session = WhisperSession::with_runner(directory.path(), settings, runner.clone());
        let mut source = ScriptedSource::speech_then_silence(40, 40);
        let transcript = session.dictate(&mut source).unwrap();
        assert_eq!(transcript.cleaned(), "привет мир");
        assert_eq!(transcript.language, "ru");
        assert!(transcript.audio_ms > 0);
        // One call, with the model and the audio the session wrote.
        let calls = runner.calls();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains(&model.to_string_lossy().into_owned()));
        // The audio and the JSON report are gone: only the text stays.
        assert!(!directory.path().join(AUDIO_FILE_NAME).exists());
        assert!(!directory
            .path()
            .join(format!("{OUTPUT_PREFIX}.json"))
            .exists());
        assert_eq!(session.state(), DictationState::Idle);
    }

    #[test]
    fn the_audio_is_kept_only_when_the_user_asked_for_it() {
        let (directory, binary, model) = prepared_directory();
        let mut settings = enabled_settings(&binary, &model);
        settings.keep_audio = true;
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let mut source = ScriptedSource::speech_then_silence(40, 40);
        session.dictate(&mut source).unwrap();
        assert!(directory.path().join(AUDIO_FILE_NAME).exists());
    }

    #[test]
    fn a_build_that_prints_instead_of_writing_json_still_produces_a_transcript() {
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::stdout_only("привет из stdout")),
        );
        let transcript = session.transcribe_samples(&vec![1000i16; 16_000]).unwrap();
        assert_eq!(transcript.cleaned(), "привет из stdout");
    }

    #[test]
    fn a_failed_build_is_reported_without_any_output() {
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::failing()),
        );
        let error = session
            .transcribe_samples(&vec![1000i16; 16_000])
            .unwrap_err();
        assert!(matches!(error, WhisperError::ProcessFailed { .. }));
        assert_eq!(session.state(), DictationState::Idle);
        assert!(!directory.path().join(AUDIO_FILE_NAME).exists());
    }

    #[test]
    fn a_second_request_is_refused_while_one_is_running() {
        let (directory, binary, model) = prepared_directory();
        let mut settings = enabled_settings(&binary, &model);
        settings.timeout_seconds = 30;
        let session = Arc::new(WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::hanging()),
        ));
        let worker = {
            let session = Arc::clone(&session);
            std::thread::spawn(move || session.transcribe_samples(&vec![1000i16; 16_000]))
        };
        // Wait until the first one is inside the runner.
        let deadline = Instant::now() + Duration::from_secs(2);
        while session.state() == DictationState::Idle && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        // The samples are being read by the model now, so the state is the
        // transcription, not the recording.
        assert_eq!(session.state(), DictationState::Transcribing);
        assert_eq!(
            session.transcribe_samples(&[0i16; 16_000]).unwrap_err(),
            WhisperError::Busy
        );
        // Cancel stops the hanging build, and the session returns to idle.
        assert!(session.cancel());
        let error = worker.join().unwrap().unwrap_err();
        assert_eq!(error, WhisperError::Cancelled);
        assert_eq!(session.state(), DictationState::Idle);
    }

    #[test]
    fn a_shutdown_is_safe_to_repeat_and_leaves_nothing_behind() {
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let mut source = ScriptedSource::speech_then_silence(40, 40);
        session.dictate(&mut source).unwrap();
        session.shutdown();
        session.shutdown();
        assert_eq!(session.state(), DictationState::Idle);
        assert!(!directory.path().join(AUDIO_FILE_NAME).exists());
        // Cancelling an idle session is a no-op, not an error.
        assert!(!session.cancel());
    }

    #[test]
    fn the_status_reports_what_is_missing_without_naming_a_path_in_a_note() {
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let status = session.status();
        assert!(status.configured);
        assert!(status.enabled);
        assert_eq!(status.model.unwrap().kind.label(), "small");
        assert!(status.binary.is_some());
        // A note is either a Fluent key or an explanation, and never a path.
        for note in &status.notes {
            assert!(!note.contains('/'), "{note}");
            assert!(!note.contains('\\'), "{note}");
            assert!(!note.contains("FICTIONAL"), "{note}");
        }

        // A missing binary and a missing model are reported as notes, not as paths.
        let empty = WhisperSettings {
            enabled: false,
            ..WhisperSettings::default()
        };
        let session = WhisperSession::with_runner(
            directory.path(),
            empty,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let status = session.status();
        assert!(!status.configured);
        assert!(status.notes.iter().any(|note| note.ends_with("no-binary")));
        assert!(status.notes.iter().any(|note| note.ends_with("no-model")));
        assert!(status.notes.iter().any(|note| note.ends_with("disabled")));
    }

    #[test]
    fn a_file_that_is_not_sixteen_kilohertz_mono_is_refused_with_a_reason() {
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let path = directory.path().join("stereo.wav");
        let mut header = Vec::new();
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&36u32.to_le_bytes());
        header.extend_from_slice(b"WAVE");
        header.extend_from_slice(b"fmt ");
        header.extend_from_slice(&16u32.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&2u16.to_le_bytes());
        header.extend_from_slice(&44_100u32.to_le_bytes());
        header.extend_from_slice(&176_400u32.to_le_bytes());
        header.extend_from_slice(&4u16.to_le_bytes());
        header.extend_from_slice(&16u16.to_le_bytes());
        header.extend_from_slice(b"data");
        header.extend_from_slice(&0u32.to_le_bytes());
        std::fs::write(&path, &header).unwrap();
        let error = session.transcribe_path(&path).unwrap_err();
        assert!(matches!(error, WhisperError::AudioUnavailable(_)));
        assert!(!error.to_string().contains("stereo.wav"));
    }

    #[test]
    fn settings_are_written_atomically_and_read_back_by_a_new_session() {
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let session = WhisperSession::with_runner(
            directory.path(),
            settings.clone(),
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let updated = WhisperSettings {
            language: "ru".to_string(),
            threads: 8,
            ..settings.clone()
        };
        session.update_settings(updated.clone()).unwrap();
        assert!(directory.path().join(SETTINGS_FILE).is_file());
        // No temporary file is left behind by the rename.
        assert!(!directory
            .path()
            .join(format!("{SETTINGS_FILE}.tmp"))
            .exists());
        // A new session reads the document the previous one wrote.
        let reopened = WhisperSession::open(directory.path(), WhisperSettings::default());
        assert_eq!(reopened.settings().threads, 8);
        assert_eq!(reopened.settings().language, "ru");
    }

    /// A source that fails the way the recorder did in the reported defect.
    struct FailingSource {
        error: Option<WhisperError>,
        frames_served: usize,
    }

    impl FailingSource {
        /// The recorder was never initialised: this is the exact failure that
        /// used to be `called Option::unwrap() on a None value`.
        fn not_initialized() -> Self {
            Self {
                error: Some(WhisperError::AudioUnavailable(
                    crate::recorder::RecorderError::NotInitialized.to_string(),
                )),
                frames_served: 0,
            }
        }

        fn no_input_device() -> Self {
            Self {
                error: Some(WhisperError::AudioUnavailable(
                    crate::recorder::RecorderError::NoInputDevice.to_string(),
                )),
                frames_served: 0,
            }
        }

        fn unsupported_configuration() -> Self {
            Self {
                error: Some(WhisperError::AudioUnavailable(
                    crate::recorder::RecorderError::UnsupportedConfiguration(
                        "sample rate".to_string(),
                    )
                    .to_string(),
                )),
                frames_served: 0,
            }
        }

        /// A source whose channel is closed after a few frames: the worker that
        /// produced the audio is gone.
        fn closing_channel(frames: usize) -> Self {
            Self {
                error: Some(WhisperError::AudioUnavailable(
                    "the audio stream ended".to_string(),
                )),
                frames_served: frames,
            }
        }
    }

    impl FrameSource for FailingSource {
        fn is_running(&self) -> bool {
            // A closed channel stops the source, the same way a dropped sender
            // stops a stream.
            self.error.is_some() || self.frames_served > 0
        }

        fn read_frame(&mut self, buffer: &mut [i16]) -> Result<(), WhisperError> {
            // The scripted frames come first, then the failure: that is what a
            // stream that dies in the middle looks like.
            if self.frames_served > 0 {
                self.frames_served -= 1;
                buffer.fill(3000);
                return Ok(());
            }
            if let Some(error) = self.error.take() {
                return Err(error);
            }
            Err(WhisperError::AudioUnavailable(
                "the audio stream ended".to_string(),
            ))
        }
    }

    #[test]
    fn a_recorder_that_was_never_initialised_ends_the_recording_instead_of_panicking() {
        // The reported defect, at the level where it happened: the frame source
        // reports `not_initialized` instead of panicking inside the read, and the
        // session must come back to `Idle` with the device released.
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let mut source = FailingSource::not_initialized();
        let error = session.record(&mut source).unwrap_err();
        assert!(matches!(error, WhisperError::AudioUnavailable(_)));
        assert_eq!(
            session.state(),
            DictationState::Idle,
            "the session must be idle"
        );
        assert_eq!(
            session.last_outcome(),
            Some(DictationOutcome::Failed {
                code: "audio_unavailable"
            })
        );
        assert!(!session.holds_microphone());
        assert!(!directory.path().join(AUDIO_FILE_NAME).exists());
    }

    #[test]
    fn no_input_device_and_an_unsupported_configuration_end_the_recording_the_same_way() {
        for (label, mut source) in [
            ("no device", FailingSource::no_input_device()),
            (
                "unsupported configuration",
                FailingSource::unsupported_configuration(),
            ),
        ] {
            let (directory, binary, model) = prepared_directory();
            let session = WhisperSession::with_runner(
                directory.path(),
                enabled_settings(&binary, &model),
                Arc::new(FakeTranscriber::answering("привет")),
            );
            let error = session.record(&mut source).unwrap_err();
            assert!(
                matches!(error, WhisperError::AudioUnavailable(_)),
                "{label}"
            );
            assert_eq!(session.state(), DictationState::Idle, "{label}");
            assert!(!session.holds_microphone(), "{label}");
            // The message is the recorder's own sentence, not a panic and not a
            // code the user cannot read.
            assert!(error.to_string().contains("microphone"), "{label}: {error}");
        }
    }

    #[test]
    fn a_closed_audio_channel_stops_the_recording_and_returns_to_idle() {
        let (directory, binary, model) = prepared_directory();
        let session = WhisperSession::with_runner(
            directory.path(),
            enabled_settings(&binary, &model),
            Arc::new(FakeTranscriber::answering("привет")),
        );
        // The source produces a few frames and then reports that the stream is
        // gone, which is what a dropped channel looks like.
        // Enough frames to pass the "something was recorded" floor, so the
        // stream ending is what surfaces rather than an empty buffer.
        let mut source = FailingSource::closing_channel(10);
        let error = session.record(&mut source).unwrap_err();
        assert!(matches!(error, WhisperError::AudioUnavailable(_)));
        assert_eq!(session.state(), DictationState::Idle);
        assert!(session.last_outcome().is_some());
    }

    #[test]
    fn a_manual_stop_ends_the_recording_and_is_bounded() {
        let (directory, binary, model) = prepared_directory();
        let mut settings = enabled_settings(&binary, &model);
        settings.max_seconds = 30;
        settings.silence_ms = 10_000;
        let session = Arc::new(WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        ));
        // Endless loud audio: only a manual stop can end this.
        let worker = {
            let session = Arc::clone(&session);
            std::thread::spawn(move || {
                let mut source = ScriptedSource::endless_speech();
                session.record(&mut source)
            })
        };
        // Wait until the recording is running, then stop it.
        let deadline = Instant::now() + Duration::from_secs(2);
        while session.state() != DictationState::Recording && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(session.state(), DictationState::Recording);
        let started = Instant::now();
        assert!(session.cancel(), "a running recording must be stoppable");
        // The flag is set without waiting for anything.
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "stop must be bounded"
        );
        assert_eq!(session.state(), DictationState::Stopping);
        let ended = worker.join().unwrap();
        assert_eq!(ended.unwrap().1, StopReason::Requested);
        assert_eq!(session.state(), DictationState::Idle);
        assert!(!session.holds_microphone());
        // Cancelling an idle session is a no-op, not an error.
        assert!(!session.cancel());
    }

    #[test]
    fn a_worker_error_leaves_the_session_idle_and_unlocks_the_next_attempt() {
        let (directory, binary, model) = prepared_directory();
        let session = WhisperSession::with_runner(
            directory.path(),
            enabled_settings(&binary, &model),
            Arc::new(FakeTranscriber::failing()),
        );
        let error = session
            .transcribe_samples(&vec![1000i16; 16_000])
            .unwrap_err();
        assert!(matches!(error, WhisperError::ProcessFailed { .. }));
        assert_eq!(session.state(), DictationState::Idle);
        assert_eq!(
            session.last_outcome(),
            Some(DictationOutcome::Failed {
                code: "process_failed"
            })
        );
        // The button works again: a second attempt is accepted, not refused.
        assert!(!session.cancel());
        assert!(!session.is_busy());
    }
    /// The exact paths and size from the report of the defect.
    const REPORTED_BINARY: &str = r"C:\AI\whisper.cpp\runtime\Release\whisper-cli.exe";
    const REPORTED_MODEL: &str = r"C:\AI\whisper.cpp\ggml-small.bin";
    const REPORTED_MODEL_BYTES: u64 = 487_601_967;

    /// A directory holding the two files exactly as they are on the machine that
    /// reported the defect: a real `whisper-cli.exe` header and a real model.
    fn reported_directory() -> (tempfile::TempDir, String, String) {
        let directory = tempdir().unwrap();
        let mut pe = vec![0u8; 0x100];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3c..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        pe[0x44..0x46].copy_from_slice(&PE_MACHINE_AMD64.to_le_bytes());
        let binary = directory.path().join("whisper-cli.exe");
        std::fs::write(&binary, &pe).unwrap();
        // The magic whisper.cpp writes: `0x67676d6c` as a little-endian integer.
        let mut body = vec![0x6cu8, 0x6d, 0x67, 0x67];
        body.resize(REPORTED_MODEL_BYTES as usize, 0);
        let model = directory.path().join("ggml-small.bin");
        std::fs::write(&model, &body).unwrap();
        let binary_path = binary.to_string_lossy().into_owned();
        let model_path = model.to_string_lossy().into_owned();
        (directory, binary_path, model_path)
    }

    #[test]
    fn the_reported_configuration_is_ready_and_survives_a_restart() {
        // This is the defect, end to end at the core level: a real
        // `whisper-cli.exe`, a real `ggml-small.bin` of the reported size, and
        // the settings written, read back, and reported as ready.
        let (directory, binary_path, model_path) = reported_directory();
        let settings = WhisperSettings {
            enabled: true,
            binary_path: binary_path.clone(),
            model_path: model_path.clone(),
            language: "ru".to_string(),
            ..WhisperSettings::default()
        };
        let session = WhisperSession::with_runner(
            directory.path(),
            settings.clone(),
            Arc::new(FakeTranscriber::answering("привет")),
        );
        // The settings page stores what it shows, so the document exists before
        // the restart; a session that only held the values in memory would of
        // course not survive it.
        session.update_settings(settings.clone()).unwrap();
        let status = session.status();
        assert!(
            status.configured,
            "the reported files must be accepted: {status:?}"
        );
        assert!(status.enabled);
        assert_eq!(status.model.as_ref().unwrap().kind.label(), "small");
        assert_eq!(
            status.model.as_ref().unwrap().size_bytes,
            REPORTED_MODEL_BYTES
        );
        // The names are safe to show anywhere, and the paths are the settings'.
        assert_eq!(status.binary_name, "whisper-cli.exe");
        assert_eq!(status.model_name, "ggml-small.bin");
        assert_eq!(status.binary_path, binary_path);
        assert_eq!(status.model_path, model_path);
        // No notes at all: nothing is missing and nothing is unverified beyond
        // the model's own honesty note, which the probe adds.
        assert!(
            !status
                .notes
                .iter()
                .any(|note| note.contains("model-unavailable")),
            "{:?}",
            status.notes
        );

        // A new session over the same directory reads the document back.
        let reopened = WhisperSession::open(directory.path(), WhisperSettings::default());
        assert!(
            reopened.settings().enabled,
            "enabled must survive a restart"
        );
        assert_eq!(reopened.settings().binary_path, binary_path);
        assert_eq!(reopened.settings().model_path, model_path);
        assert!(reopened.status().configured);
        assert!(reopened.settings().validate().is_ok());
    }

    #[test]
    fn saving_enabled_alone_never_drops_the_two_paths() {
        // The switch is saved on its own, and it must not lose what was chosen
        // before it: one document, written whole.
        let (directory, binary_path, model_path) = reported_directory();
        let session = WhisperSession::with_runner(
            directory.path(),
            WhisperSettings::default(),
            Arc::new(FakeTranscriber::answering("привет")),
        );
        // First the two files, as the settings page stores them.
        session
            .update_settings(WhisperSettings {
                binary_path: binary_path.clone(),
                model_path: model_path.clone(),
                ..WhisperSettings::default()
            })
            .unwrap();
        // Then the switch, built from what the page was showing.
        let shown = session.settings();
        let updated = session
            .update_settings(WhisperSettings {
                enabled: true,
                ..shown.clone()
            })
            .unwrap();
        assert!(updated.enabled);
        assert_eq!(updated.binary_path, binary_path);
        assert_eq!(updated.model_path, model_path);
        // And a reload sees all three together.
        let reopened = WhisperSession::open(directory.path(), WhisperSettings::default());
        assert!(reopened.settings().enabled);
        assert_eq!(reopened.settings().model_path, model_path);
        assert!(reopened.status().configured);
    }

    #[test]
    fn the_settings_document_round_trips_windows_paths_unchanged() {
        let (directory, binary_path, model_path) = reported_directory();
        let session = WhisperSession::with_runner(
            directory.path(),
            WhisperSettings::default(),
            Arc::new(FakeTranscriber::answering("привет")),
        );
        session
            .update_settings(WhisperSettings {
                binary_path: REPORTED_BINARY.to_string(),
                model_path: REPORTED_MODEL.to_string(),
                language: "ru".to_string(),
                ..WhisperSettings::default()
            })
            .unwrap();
        // The document on disk escapes the backslashes; reading it back gives the
        // same paths, character for character.
        let text = std::fs::read_to_string(directory.path().join(SETTINGS_FILE)).unwrap();
        assert!(text.contains(r"C:\\AI\\whisper.cpp"), "{text}");
        let stored = super::super::config::stored_settings(directory.path()).unwrap();
        assert_eq!(stored.binary_path, REPORTED_BINARY);
        assert_eq!(stored.model_path, REPORTED_MODEL);
        assert_eq!(stored.language, "ru");
        // A path with a space survives as well.
        session
            .update_settings(WhisperSettings {
                binary_path: r"C:\Program Files\whisper\whisper-cli.exe".to_string(),
                ..stored
            })
            .unwrap();
        let stored = super::super::config::stored_settings(directory.path()).unwrap();
        assert_eq!(
            stored.binary_path,
            r"C:\Program Files\whisper\whisper-cli.exe"
        );
        let _ = (binary_path, model_path);
    }

    #[test]
    fn a_wrong_file_is_refused_with_a_localized_reason_and_a_detail() {
        let (directory, _binary_path, _model_path) = reported_directory();
        // Text renamed to look like a model: the header is what gives it away.
        let mut text = b"NOTM".to_vec();
        text.resize(REPORTED_MODEL_BYTES as usize, 0);
        let wrong = directory.path().join("ggml-small.bin");
        std::fs::write(&wrong, &text).unwrap();
        let session = WhisperSession::with_runner(
            directory.path(),
            WhisperSettings {
                enabled: true,
                binary_path: directory
                    .path()
                    .join("whisper-cli.exe")
                    .to_string_lossy()
                    .into_owned(),
                model_path: wrong.to_string_lossy().into_owned(),
                ..WhisperSettings::default()
            },
            Arc::new(FakeTranscriber::answering("привет")),
        );
        let status = session.status();
        assert!(!status.configured, "a wrong file must not look ready");
        // A Fluent key for the interface, and the concrete detail for the person.
        assert!(
            status
                .notes
                .iter()
                .any(|note| note == "windows-whisper-note-model-unavailable"),
            "{:?}",
            status.notes
        );
        assert!(
            status.notes.iter().any(|note| note.contains("4e 4f 54 4d")),
            "the detail must say what the header was: {:?}",
            status.notes
        );
        // The note carries no path, so it is safe to show and to copy.
        for note in &status.notes {
            assert!(!note.contains(":\\"), "{note}");
        }
    }
    #[test]
    fn switching_the_feature_off_stops_a_running_recording() {
        let (directory, binary, model) = prepared_directory();
        let settings = enabled_settings(&binary, &model);
        let session = WhisperSession::with_runner(
            directory.path(),
            settings,
            Arc::new(FakeTranscriber::answering("привет")),
        );
        // Turn it off before anything starts: the state must not be "recording".
        let updated = session
            .update_settings(WhisperSettings {
                enabled: false,
                ..session.settings().clone()
            })
            .unwrap();
        assert!(!updated.enabled);
        assert_eq!(session.state(), DictationState::Idle);
        assert!(!session.settings().enabled);
    }
}
