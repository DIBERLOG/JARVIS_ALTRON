//! Failures of the local Whisper dictation path.
//!
//! A variant never carries a transcript, an audio buffer, a prompt, or a raw
//! process output, so an error can be shown to the user and written to a log
//! without leaking what was said or what was read out loud.

use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WhisperError {
    /// The feature is switched off in the settings.
    Disabled,
    /// No usable configuration: the binary, the model, or both are missing.
    NotConfigured,
    /// The configuration was rejected before anything ran.
    InvalidConfiguration(String),
    /// The binary path is not a usable executable.
    BinaryUnavailable(String),
    /// The model file was rejected: missing, empty, or not a Whisper model.
    ModelUnavailable(String),
    /// The model file is a Whisper model but not one this build recognizes by
    /// name, so its size and accuracy cannot be described.
    ModelUnknown,
    /// The executable was built for another architecture.
    WrongArchitecture {
        expected: &'static str,
        found: String,
    },
    /// The audio could not be prepared (written, read, or decoded).
    AudioUnavailable(String),
    /// The microphone could not be used, with the recorder's own code.
    ///
    /// The code is one of `not_initialized`, `no_input_device`,
    /// `unsupported_configuration`, `device_failed`, `backend_unavailable`,
    /// `permission_denied`, `already_running`, or `not_running`: the interface
    /// shows it instead of a generic "audio unavailable".
    RecorderUnavailable { code: String },
    /// The recording is too short or too quiet to transcribe.
    AudioEmpty,
    /// Another transcription is already running in this session.
    Busy,
    /// The process could not be spawned, for example because of permissions.
    ProcessUnavailable,
    /// The process exited with a failure.
    ProcessFailed { code: Option<i32> },
    /// The process did not finish within the configured timeout and was stopped.
    TimedOut,
    /// The process produced nothing that could be read as a transcript.
    InvalidResponse,
    /// The user cancelled the transcription.
    Cancelled,
    /// The language is not one this build offers.
    UnsupportedLanguage(String),
    /// Writing or reading a file in the feature's own directory failed.
    Storage,
}

impl WhisperError {
    /// A stable, content-free code for the interface and the log.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NotConfigured => "not_configured",
            Self::InvalidConfiguration(_) => "invalid_configuration",
            Self::BinaryUnavailable(_) => "binary_unavailable",
            Self::ModelUnavailable(_) => "model_unavailable",
            Self::ModelUnknown => "model_unknown",
            Self::WrongArchitecture { .. } => "wrong_architecture",
            Self::AudioUnavailable(_) => "audio_unavailable",
            Self::RecorderUnavailable { .. } => "recorder_unavailable",
            Self::AudioEmpty => "audio_empty",
            Self::Busy => "busy",
            Self::ProcessUnavailable => "process_unavailable",
            Self::ProcessFailed { .. } => "process_failed",
            Self::TimedOut => "timed_out",
            Self::InvalidResponse => "invalid_response",
            Self::Cancelled => "cancelled",
            Self::UnsupportedLanguage(_) => "unsupported_language",
            Self::Storage => "storage",
        }
    }

    /// Whether the user can fix this by changing a setting or picking a file.
    pub fn is_configuration_problem(&self) -> bool {
        matches!(
            self,
            Self::NotConfigured
                | Self::InvalidConfiguration(_)
                | Self::BinaryUnavailable(_)
                | Self::ModelUnavailable(_)
                | Self::ModelUnknown
                | Self::WrongArchitecture { .. }
                | Self::UnsupportedLanguage(_)
        )
    }

    /// Whether the failure came from the user stopping the work.
    pub fn is_cancellation(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// The content-free detail of a variant, when it has one.
    ///
    /// The detail is what turns "that file cannot be used" into a sentence a
    /// person can act on. Every detail in this type is a static sentence or a
    /// bounded number, so it can be shown and logged: none of them carries a
    /// path, a transcript, or an audio buffer.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::InvalidConfiguration(detail)
            | Self::BinaryUnavailable(detail)
            | Self::ModelUnavailable(detail)
            | Self::AudioUnavailable(detail) => Some(detail.clone()),
            Self::RecorderUnavailable { code } => Some(code.clone()),
            Self::WrongArchitecture { expected, found } => {
                Some(format!("built for {found}, needs {expected}"))
            }
            Self::ProcessFailed { code } => code.map(|code| format!("exit code {code}")),
            Self::UnsupportedLanguage(language) => Some(format!("language {language}")),
            _ => None,
        }
    }
}

impl fmt::Display for WhisperError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("dictation is switched off in the settings"),
            Self::NotConfigured => {
                formatter.write_str("pick a whisper executable and a model first")
            }
            Self::InvalidConfiguration(detail) => {
                write!(formatter, "the dictation settings were rejected: {detail}")
            }
            Self::BinaryUnavailable(detail) => {
                write!(formatter, "the whisper executable cannot be used: {detail}")
            }
            Self::ModelUnavailable(detail) => {
                write!(formatter, "the model file cannot be used: {detail}")
            }
            Self::ModelUnknown => formatter.write_str(
                "the model file is a Whisper model, but its size is not recognizable from its name",
            ),
            Self::WrongArchitecture { expected, found } => write!(
                formatter,
                "the executable is built for {found}, but this build needs {expected}"
            ),
            Self::AudioUnavailable(detail) => {
                write!(formatter, "the audio cannot be used: {detail}")
            }
            Self::RecorderUnavailable { code } => {
                write!(formatter, "the microphone is not available ({code})")
            }
            Self::AudioEmpty => formatter.write_str("nothing was recorded"),
            Self::Busy => formatter.write_str("a transcription is already running"),
            Self::ProcessUnavailable => {
                formatter.write_str("the whisper process could not be started")
            }
            Self::ProcessFailed { code } => match code {
                Some(code) => write!(formatter, "the whisper process failed with code {code}"),
                None => formatter.write_str("the whisper process failed"),
            },
            Self::TimedOut => {
                formatter.write_str("the transcription took too long and was stopped")
            }
            Self::InvalidResponse => {
                formatter.write_str("the whisper process produced no readable transcript")
            }
            Self::Cancelled => formatter.write_str("the transcription was cancelled"),
            Self::UnsupportedLanguage(language) => {
                write!(formatter, "the language {language} is not offered")
            }
            Self::Storage => formatter.write_str("the feature could not write its own files"),
        }
    }
}

impl std::error::Error for WhisperError {}

impl From<std::io::Error> for WhisperError {
    fn from(_: std::io::Error) -> Self {
        // The io error's message can carry a path; the path is not kept.
        Self::Storage
    }
}

impl From<serde_json::Error> for WhisperError {
    fn from(_: serde_json::Error) -> Self {
        Self::InvalidResponse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<WhisperError> {
        vec![
            WhisperError::Disabled,
            WhisperError::NotConfigured,
            WhisperError::InvalidConfiguration("threads must be 1 to 16".to_string()),
            WhisperError::BinaryUnavailable("not a file".to_string()),
            WhisperError::ModelUnavailable("empty".to_string()),
            WhisperError::ModelUnknown,
            WhisperError::WrongArchitecture {
                expected: "x86-64",
                found: "x86".to_string(),
            },
            WhisperError::AudioUnavailable("not a wav".to_string()),
            WhisperError::RecorderUnavailable {
                code: "not_initialized".to_string(),
            },
            WhisperError::AudioEmpty,
            WhisperError::Busy,
            WhisperError::ProcessUnavailable,
            WhisperError::ProcessFailed { code: Some(1) },
            WhisperError::TimedOut,
            WhisperError::InvalidResponse,
            WhisperError::Cancelled,
            WhisperError::UnsupportedLanguage("xx".to_string()),
            WhisperError::Storage,
        ]
    }

    #[test]
    fn every_variant_has_its_own_code() {
        let mut codes: Vec<&'static str> =
            every_variant().iter().map(|error| error.code()).collect();
        codes.sort_unstable();
        let unique = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), unique, "two variants share a code");
        assert!(codes.iter().all(|code| !code.is_empty()));
    }

    #[test]
    fn no_message_carries_what_was_said_or_the_path() {
        // The details below stand in for a file path and for a transcript; neither may reach a
        // log or the interface through an error.
        for error in every_variant() {
            let rendered = error.to_string();
            assert!(!rendered.contains("FICTIONAL"), "{rendered}");
            assert!(!rendered.contains('/'), "{rendered}");
            assert!(!rendered.contains('\\'), "{rendered}");
        }
    }

    #[test]
    fn problems_the_user_can_fix_are_marked_as_such() {
        assert!(WhisperError::NotConfigured.is_configuration_problem());
        assert!(WhisperError::ModelUnknown.is_configuration_problem());
        assert!(!WhisperError::Busy.is_configuration_problem());
        assert!(!WhisperError::Storage.is_configuration_problem());
        assert!(WhisperError::Cancelled.is_cancellation());
        assert!(!WhisperError::TimedOut.is_cancellation());
    }

    #[test]
    fn an_io_failure_becomes_a_content_free_storage_error() {
        let error: WhisperError =
            std::io::Error::new(std::io::ErrorKind::PermissionDenied, "C:/secret/place").into();
        assert_eq!(error, WhisperError::Storage);
        assert_eq!(error.code(), "storage");
    }
}
