//! Why a global dictation request did not produce text where it was asked to.
//!
//! Every variant is a stable code and a sentence with no content in it: no
//! transcript, no field title, no document name, no window title, no path.

use std::fmt;

/// What went wrong.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DictationError {
    /// The feature is switched off in the settings.
    Disabled,
    /// A request is already running.
    Busy,
    /// The voice host could not give up the microphone.
    MicrophoneBusy(&'static str),
    /// The voice host could not take the microphone back.
    VoiceHostUnavailable,
    /// The microphone is not usable, with the recorder's own code.
    RecorderUnavailable {
        stage: &'static str,
        code: &'static str,
    },
    /// Nothing was recorded, or nothing was recognized.
    EmptyRecording,
    /// The transcription failed, with the whisper code.
    TranscriptionFailed(&'static str),
    /// The window that had the focus is gone, or a different one has it now.
    WindowChanged,
    /// The focused element may not receive text, with the rule that refused it.
    TargetRefused(&'static str),
    /// UI Automation could not be used, and the clipboard was not allowed either.
    NoDeliveryPath,
    /// The text was copied instead of inserted, because nothing else was possible.
    CopiedToClipboard,
    /// The person cancelled before the text was delivered.
    Cancelled,
    /// A part of the route was not reachable at all.
    Unavailable(&'static str),
}

impl DictationError {
    /// A stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Busy => "busy",
            Self::MicrophoneBusy(_) => "microphone_busy",
            Self::VoiceHostUnavailable => "voice_host_unavailable",
            Self::RecorderUnavailable { .. } => "recorder_unavailable",
            Self::EmptyRecording => "audio_empty",
            Self::TranscriptionFailed(_) => "transcription_failed",
            Self::WindowChanged => "window_changed",
            Self::TargetRefused(_) => "target_refused",
            Self::NoDeliveryPath => "no_delivery_path",
            Self::CopiedToClipboard => "copied_to_clipboard",
            Self::Cancelled => "cancelled",
            Self::Unavailable(_) => "unavailable",
        }
    }

    /// The rule or the stage behind the code, when there is one.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::MicrophoneBusy(detail)
            | Self::TranscriptionFailed(detail)
            | Self::TargetRefused(detail)
            | Self::Unavailable(detail) => Some((*detail).to_string()),
            Self::RecorderUnavailable { stage, code } => Some(format!("{stage}: {code}")),
            _ => None,
        }
    }
}

impl fmt::Display for DictationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => formatter.write_str("global voice input is switched off"),
            Self::Busy => formatter.write_str("a voice input request is already running"),
            Self::MicrophoneBusy(detail) => {
                write!(
                    formatter,
                    "the microphone could not be handed over: {detail}"
                )
            }
            Self::VoiceHostUnavailable => {
                formatter.write_str("the listener could not take the microphone back")
            }
            Self::RecorderUnavailable { stage, code } => {
                write!(
                    formatter,
                    "the microphone is not available ({stage}: {code})"
                )
            }
            Self::EmptyRecording => formatter.write_str("nothing was recognized"),
            Self::TranscriptionFailed(code) => {
                write!(formatter, "the transcription failed ({code})")
            }
            Self::WindowChanged => {
                formatter.write_str("the active window changed, so nothing was typed")
            }
            Self::TargetRefused(rule) => {
                write!(formatter, "that field may not receive text ({rule})")
            }
            Self::NoDeliveryPath => {
                formatter.write_str("there was no safe way to deliver the text")
            }
            Self::CopiedToClipboard => {
                formatter.write_str("the text is on the clipboard, ready to paste")
            }
            Self::Cancelled => formatter.write_str("the request was cancelled"),
            Self::Unavailable(what) => write!(formatter, "{what} is not available"),
        }
    }
}

impl std::error::Error for DictationError {}

impl From<crate::recorder::RecorderError> for DictationError {
    fn from(error: crate::recorder::RecorderError) -> Self {
        Self::RecorderUnavailable {
            stage: error.stage(),
            code: error.code(),
        }
    }
}

impl From<crate::whisper::WhisperError> for DictationError {
    fn from(error: crate::whisper::WhisperError) -> Self {
        Self::TranscriptionFailed(error.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<DictationError> {
        vec![
            DictationError::Disabled,
            DictationError::Busy,
            DictationError::MicrophoneBusy("vosk"),
            DictationError::VoiceHostUnavailable,
            DictationError::RecorderUnavailable {
                stage: "claim",
                code: "recorder_busy",
            },
            DictationError::EmptyRecording,
            DictationError::TranscriptionFailed("process_failed"),
            DictationError::WindowChanged,
            DictationError::TargetRefused("password_field"),
            DictationError::NoDeliveryPath,
            DictationError::CopiedToClipboard,
            DictationError::Cancelled,
            DictationError::Unavailable("ui_automation"),
        ]
    }

    #[test]
    fn every_variant_has_its_own_code() {
        let mut codes: Vec<&'static str> =
            every_variant().iter().map(DictationError::code).collect();
        codes.sort_unstable();
        let unique = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), unique, "two variants share a code");
        assert!(codes.iter().all(|code| !code.is_empty()));
    }

    #[test]
    fn no_message_carries_a_path_or_a_sentence_of_the_user() {
        for error in every_variant() {
            let rendered = format!("{error} {:?}", error);
            for forbidden in ["FICTIONAL", "\\", "/", "C:"] {
                assert!(
                    !rendered.contains(forbidden),
                    "{forbidden} must not appear in {rendered}"
                );
            }
        }
    }
}
