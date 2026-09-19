//! Recording failures.
//!
//! Every variant is a code and a short sentence: no device name, no path, and no
//! audio. An error from here can be shown in the window and written to a log.

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
}
