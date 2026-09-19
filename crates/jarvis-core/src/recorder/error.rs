//! Recording failures.
//!
//! Every variant is a code and a short sentence: no device name, no path, and no
//! audio. An error from here can be shown in the window and written to a log.
//!
//! Two things are deliberately separated:
//!
//! * **where it failed** — [`RecorderError::stage`] is `init`, `claim`, `start`,
//!   `read`, or `state`. The window shows the code; the log keeps the stage, so
//!   "the microphone is unavailable" stops being one answer for five different
//!   failures;
//! * **who is holding the device** — [`RecorderError::Busy`] and
//!   [`RecorderError::VoiceOwnsMicrophone`] name the other owner instead of
//!   reporting a device problem that does not exist.

use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecorderError {
    /// The recorder was never initialised in this process.
    ///
    /// This is the failure that used to be a panic: the cells that hold the
    /// backend and the frame length are empty until `recorder::init()` runs, and
    /// a process that never called it (the window is one) reached the native
    /// read with an empty cell.
    NotInitialized,
    /// No input device is available.
    NoInputDevice,
    /// The device exists and this build cannot use its configuration.
    UnsupportedConfiguration(String),
    /// The device or the backend failed while recording.
    DeviceFailed(String),
    /// A recording is already running in this process.
    AlreadyRunning,
    /// Nothing is recording, so there is nothing to stop.
    NotRunning,
    /// The backend is not implemented in this build.
    BackendUnavailable,
    /// The operating system refused access to the microphone.
    ///
    /// This is the Windows privacy setting and the device-in-use case. It is its
    /// own variant because the answer for the user is different: no amount of
    /// retrying helps until the setting is changed.
    PermissionDenied(String),
    /// Another part of the application is holding the microphone right now.
    ///
    /// `held_by` is one of the fixed owner names — `a microphone check`,
    /// `a dictation`, `the voice listener` — and never a device or a person.
    Busy { held_by: &'static str },
    /// The wake-word listener owns the microphone.
    ///
    /// Its own code because the answer is "stop listening first", not "your
    /// microphone is broken".
    VoiceOwnsMicrophone,
    /// The stream was open and the native start refused.
    ///
    /// The inner code is kept, so an actionable failure — access denied, no
    /// device, an unimplemented backend — is still recognizable underneath.
    StartFailed { inner: &'static str },
    /// The stream is open and the frame could not be read.
    ReadFailed { inner: &'static str },
    /// The recorder was asked for something that its state does not allow.
    ///
    /// The detail is a fixed sentence: "the stream is not open" is the common
    /// one, and it is what a read before a start has to say.
    InvalidState(&'static str),
}

impl RecorderError {
    /// A stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotInitialized => "not_initialized",
            Self::NoInputDevice => "no_input_device",
            Self::UnsupportedConfiguration(_) => "unsupported_configuration",
            Self::DeviceFailed(_) => "device_failed",
            Self::AlreadyRunning => "already_running",
            Self::NotRunning => "not_running",
            Self::BackendUnavailable => "backend_unavailable",
            Self::PermissionDenied(_) => "permission_denied",
            Self::Busy { .. } => "recorder_busy",
            Self::VoiceOwnsMicrophone => "vosk_owns_microphone",
            Self::StartFailed { .. } => "start_failed",
            Self::ReadFailed { .. } => "read_failed",
            Self::InvalidState(_) => "invalid_state",
        }
    }

    /// Where in the recording the failure happened.
    ///
    /// The window shows the code; the log keeps the stage as well, because
    /// "the microphone is unavailable" from five different stages is not a
    /// diagnosis. The stage is a fixed word and carries no content.
    pub fn stage(&self) -> &'static str {
        match self {
            Self::NotInitialized
            | Self::NoInputDevice
            | Self::UnsupportedConfiguration(_)
            | Self::BackendUnavailable
            | Self::PermissionDenied(_) => "init",
            Self::Busy { .. } | Self::VoiceOwnsMicrophone | Self::AlreadyRunning => "claim",
            Self::StartFailed { .. } => "start",
            Self::ReadFailed { .. } | Self::DeviceFailed(_) => "read",
            Self::NotRunning | Self::InvalidState(_) => "state",
        }
    }

    /// The fixed owner name of a busy microphone, when that is the answer.
    pub fn held_by(&self) -> Option<&'static str> {
        match self {
            Self::Busy { held_by } => Some(held_by),
            Self::VoiceOwnsMicrophone => Some(crate::recorder::owner_name(
                crate::recorder::MicrophoneOwner::Voice,
            )),
            _ => None,
        }
    }

    /// A short, content-free sentence about the failure, when it has one.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::UnsupportedConfiguration(detail)
            | Self::DeviceFailed(detail)
            | Self::PermissionDenied(detail) => Some(detail.clone()),
            Self::InvalidState(state) => Some((*state).to_string()),
            Self::StartFailed { inner } | Self::ReadFailed { inner } => {
                Some(format!("the recorder answered {inner}"))
            }
            _ => None,
        }
    }

    /// Whether the user can fix this by choosing a different device.
    pub fn is_configuration_problem(&self) -> bool {
        matches!(
            self,
            Self::NoInputDevice
                | Self::UnsupportedConfiguration(_)
                | Self::BackendUnavailable
                | Self::PermissionDenied(_)
        )
    }
}

impl fmt::Display for RecorderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotInitialized => {
                formatter.write_str("the recorder is not ready in this process")
            }
            Self::NoInputDevice => formatter.write_str("no microphone is available"),
            Self::UnsupportedConfiguration(detail) => {
                write!(formatter, "this microphone cannot be used: {detail}")
            }
            Self::DeviceFailed(detail) => write!(formatter, "the microphone failed: {detail}"),
            Self::AlreadyRunning => formatter.write_str("a recording is already running"),
            Self::NotRunning => formatter.write_str("nothing is recording"),
            Self::BackendUnavailable => {
                formatter.write_str("this build has no recording backend for that device")
            }
            Self::PermissionDenied(detail) => {
                write!(
                    formatter,
                    "the system refused access to the microphone: {detail}"
                )
            }
            Self::Busy { held_by } => {
                write!(formatter, "the microphone is already in use by {held_by}")
            }
            Self::VoiceOwnsMicrophone => {
                formatter.write_str("the voice listener is using the microphone")
            }
            Self::StartFailed { inner } => {
                write!(
                    formatter,
                    "the microphone stream could not be started ({inner})"
                )
            }
            Self::ReadFailed { inner } => {
                write!(
                    formatter,
                    "the microphone stream could not be read ({inner})"
                )
            }
            Self::InvalidState(state) => {
                write!(formatter, "the recorder is in the wrong state: {state}")
            }
        }
    }
}

impl std::error::Error for RecorderError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<RecorderError> {
        vec![
            RecorderError::NotInitialized,
            RecorderError::NoInputDevice,
            RecorderError::UnsupportedConfiguration("sample rate".to_string()),
            RecorderError::DeviceFailed("read".to_string()),
            RecorderError::AlreadyRunning,
            RecorderError::NotRunning,
            RecorderError::BackendUnavailable,
            RecorderError::PermissionDenied("access".to_string()),
            RecorderError::Busy {
                held_by: "a microphone check",
            },
            RecorderError::VoiceOwnsMicrophone,
            RecorderError::StartFailed {
                inner: "device_failed",
            },
            RecorderError::ReadFailed {
                inner: "device_failed",
            },
            RecorderError::InvalidState("the stream is not open"),
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
    }

    #[test]
    fn no_message_carries_a_device_or_a_path() {
        for error in every_variant() {
            let rendered = error.to_string();
            assert!(!rendered.contains("FICTIONAL"), "{rendered}");
            assert!(!rendered.contains('/'), "{rendered}");
            assert!(!rendered.contains('\\'), "{rendered}");
        }
    }

    #[test]
    fn the_failures_a_person_can_fix_are_marked_as_such() {
        assert!(RecorderError::NoInputDevice.is_configuration_problem());
        assert!(RecorderError::BackendUnavailable.is_configuration_problem());
        assert!(RecorderError::PermissionDenied("access".to_string()).is_configuration_problem());
        assert!(!RecorderError::NotInitialized.is_configuration_problem());
        assert!(!RecorderError::DeviceFailed("x".to_string()).is_configuration_problem());
    }

    /// A stage is what turns "the microphone is unavailable" into a diagnosis.
    #[test]
    fn every_failure_names_the_stage_it_happened_in() {
        for error in every_variant() {
            let stage = error.stage();
            assert!(
                ["init", "claim", "start", "read", "state"].contains(&stage),
                "{stage} is not a known stage"
            );
        }
        assert_eq!(
            RecorderError::InvalidState("the stream is not open").stage(),
            "state"
        );
        assert_eq!(
            RecorderError::StartFailed {
                inner: "device_failed"
            }
            .stage(),
            "start"
        );
        assert_eq!(
            RecorderError::ReadFailed {
                inner: "device_failed"
            }
            .stage(),
            "read"
        );
    }

    #[test]
    fn a_busy_microphone_names_the_other_owner() {
        assert_eq!(
            RecorderError::Busy {
                held_by: "a dictation"
            }
            .held_by(),
            Some("a dictation")
        );
        assert_eq!(
            RecorderError::VoiceOwnsMicrophone.held_by(),
            Some(crate::recorder::owner_name(
                crate::recorder::MicrophoneOwner::Voice
            ))
        );
        assert_eq!(RecorderError::NotRunning.held_by(), None);
    }

    #[test]
    fn the_inner_code_of_a_stage_failure_survives_in_the_detail() {
        let error = RecorderError::StartFailed {
            inner: "permission_denied",
        };
        assert_eq!(error.code(), "start_failed");
        assert_eq!(
            error.detail().as_deref(),
            Some("the recorder answered permission_denied")
        );
    }
}
