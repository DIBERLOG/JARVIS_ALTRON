//! Local dictation through a Whisper build the user supplies.
//!
//! This module is the voice *input* path: it turns a recording into text on
//! this machine, with no network call, no account, and no model that came from
//! anywhere but the user's own disk.
//!
//! What it is:
//!
//! * a bounded, one-shot child process. The user picks a `whisper-cli.exe` and a
//!   `ggml-*.bin`; arguments are passed one by one, never through a shell, and
//!   only the process this session started is ever stopped;
//! * a recorder that opens the microphone only between an explicit start and an
//!   explicit stop, stops on silence or on length, and writes one temporary WAV
//!   in the feature directory;
//! * a transcript that goes to the caller. It is not logged, not stored, and not
//!   sent to the local model or to the AI memory by anything in this module.
//!
//! What it is not:
//!
//! * it is not a downloader. Nothing is fetched, and no official URL or hash is
//!   invented: a missing binary or model is a `NotConfigured` state for the
//!   wizard to ask about;
//! * it is not a converter. Only 16 kHz mono 16-bit PCM is transcribed, because
//!   that is what the recorder produces; a file in another format is reported,
//!   not decoded;
//! * it is not a verifier of the model. The file's container and the size its
//!   *name* claims are checked, and the notes say plainly that the contents are
//!   not checked against a hash;
//! * it is not a background listener. Nothing records unless a person asked for
//!   it in this session, and `enabled` is off by default.
//!
//! Wake-on-LAN is unrelated to this module and excluded from the project.

pub mod config;
pub mod error;
pub mod model;
pub mod runner;
pub mod session;
pub mod wav;

pub use config::{
    stored_settings, StoredSettingsSummary, WhisperSettings, DEFAULT_DICTATION_SECONDS,
    DEFAULT_SILENCE_MS, DEFAULT_THREADS, DEFAULT_TIMEOUT_SECONDS, LANGUAGES, MAX_DICTATION_SECONDS,
    MIN_AUDIO_MS, MIN_DICTATION_SECONDS, SAMPLE_RATE, SETTINGS_FILE, SETTINGS_SCHEMA_VERSION,
};
pub use error::WhisperError;
pub use model::{
    pe_architecture, probe_binary, probe_model, Architecture, BinaryProbe, ModelKind, ModelProbe,
};
pub use runner::{
    build_arguments, parse_json_transcript, parse_stdout_transcript, parse_timestamp_ms,
    ProcessTranscriber, RunOutcome, Transcriber, Transcript, TranscriptSegment, AUDIO_FILE_NAME,
};
pub use session::{
    DictationState, DictationStatus, FrameSource, RecorderFrames, Recording, StopReason,
    WhisperSession,
};
pub use wav::{
    frames_for_seconds, parse_wav_format, read_wav_format, samples_for_millis, samples_for_seconds,
    write_wav, WavFormat,
};

// `samples_for_seconds` and `peak_amplitude` are the two helpers a caller
// needs to size a buffer or to decide whether a frame carries speech.
pub use wav::peak_amplitude;
