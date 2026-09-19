//! Errors of the safe Windows actions.
//!
//! Every message is content-free: no window title, no reminder text, no file path, and no
//! secret ever appears here, so an error can be shown and logged safely. A platform error
//! code is kept as a number for diagnostics, because the text Windows returns is localized,
//! unbounded, and sometimes names objects the user did not ask about.

use std::fmt;

/// Everything that can go wrong while requesting, confirming, or executing an action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionError {
    /// This build or this operating system cannot perform the action at all.
    UnsupportedPlatform,
    /// The action needs a platform capability that is not available right now.
    CapabilityUnavailable { capability: String },
    /// The arguments are out of range, empty, or otherwise unusable.
    InvalidArguments { detail: String },
    /// The central policy refuses this action outright.
    ForbiddenAction { reason: String },
    /// The action must be confirmed before it can run.
    ConfirmationRequired,
    /// The confirmation was presented too long ago.
    ConfirmationExpired,
    /// The confirmation token does not belong to the pending request.
    ConfirmationMismatch,
    /// The application is not in the allowlist.
    ApplicationNotAllowed { application_id: String },
    /// The executable changed since it was added to the allowlist.
    ExecutableChanged { application_id: String },
    /// The window is not in the current list, or it is gone.
    WindowNotFound,
    /// A window identifier outlived its short validity.
    WindowExpired,
    /// No timer or reminder with that identifier exists.
    TimerNotFound,
    /// The screen could not be captured or the image could not be written.
    ScreenshotFailed { detail: String },
    /// A window that may be showing credentials is in the way of the capture.
    SensitiveWindow,
    /// A platform call failed. The code is for diagnostics, never a message from Windows.
    WindowsApiError { code: i32 },
    /// Reading or writing a local file of this feature failed.
    StorageError,
    /// Another action is in flight.
    Busy,
    /// The user cancelled.
    Cancelled,
}

impl ActionError {
    /// Stable, content-free code for the interface.
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedPlatform => "unsupported_platform",
            Self::CapabilityUnavailable { .. } => "capability_unavailable",
            Self::InvalidArguments { .. } => "invalid_arguments",
            Self::ForbiddenAction { .. } => "forbidden_action",
            Self::ConfirmationRequired => "confirmation_required",
            Self::ConfirmationExpired => "confirmation_expired",
            Self::ConfirmationMismatch => "confirmation_mismatch",
            Self::ApplicationNotAllowed { .. } => "application_not_allowed",
            Self::ExecutableChanged { .. } => "executable_changed",
            Self::WindowNotFound => "window_not_found",
            Self::WindowExpired => "window_expired",
            Self::TimerNotFound => "timer_not_found",
            Self::ScreenshotFailed { .. } => "screenshot_failed",
            Self::SensitiveWindow => "sensitive_window",
            Self::WindowsApiError { .. } => "windows_api_error",
            Self::StorageError => "storage_error",
            Self::Busy => "busy",
            Self::Cancelled => "cancelled",
        }
    }

    /// Whether this is the policy refusing the action rather than the platform failing.
    pub fn is_policy_refusal(&self) -> bool {
        matches!(self, Self::ForbiddenAction { .. })
    }

    /// Whether the failure needs a confirmation the caller did not provide.
    pub fn is_confirmation_related(&self) -> bool {
        matches!(
            self,
            Self::ConfirmationRequired | Self::ConfirmationExpired | Self::ConfirmationMismatch
        )
    }
}

impl fmt::Display for ActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("this action is not available on this system")
            }
            Self::CapabilityUnavailable { capability } => {
                write!(formatter, "this system cannot {capability} right now")
            }
            Self::InvalidArguments { detail } => {
                write!(formatter, "the request is not usable: {detail}")
            }
            Self::ForbiddenAction { reason } => {
                write!(formatter, "this action is not allowed: {reason}")
            }
            Self::ConfirmationRequired => {
                formatter.write_str("this action needs your confirmation first")
            }
            Self::ConfirmationExpired => {
                formatter.write_str("the confirmation expired; ask for the action again")
            }
            Self::ConfirmationMismatch => {
                formatter.write_str("that confirmation does not belong to the pending action")
            }
            Self::ApplicationNotAllowed { .. } => {
                formatter.write_str("that application is not in the allowed list")
            }
            Self::ExecutableChanged { .. } => formatter.write_str(
                "the application changed since it was allowed; check it again in the settings",
            ),
            Self::WindowNotFound => formatter.write_str("that window is no longer available"),
            Self::WindowExpired => {
                formatter.write_str("the window list is out of date; load the windows again")
            }
            Self::TimerNotFound => formatter.write_str("that timer or reminder no longer exists"),
            Self::ScreenshotFailed { detail } => {
                write!(formatter, "the screenshot could not be taken: {detail}")
            }
            Self::SensitiveWindow => formatter.write_str(
                "a window that may show private data is in the way; close or hide it and try again",
            ),
            Self::WindowsApiError { code } => {
                write!(formatter, "the system refused the request (code {code})")
            }
            Self::StorageError => formatter.write_str("a local file could not be read or written"),
            Self::Busy => formatter.write_str("another action is still running"),
            Self::Cancelled => formatter.write_str("the action was cancelled"),
        }
    }
}

impl std::error::Error for ActionError {}

impl From<std::io::Error> for ActionError {
    fn from(_: std::io::Error) -> Self {
        Self::StorageError
    }
}

impl From<serde_json::Error> for ActionError {
    fn from(_: serde_json::Error) -> Self {
        Self::StorageError
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_its_own_code() {
        let samples = [
            ActionError::UnsupportedPlatform,
            ActionError::CapabilityUnavailable {
                capability: "change the volume".to_string(),
            },
            ActionError::InvalidArguments {
                detail: "percent".to_string(),
            },
            ActionError::ForbiddenAction {
                reason: "shell".to_string(),
            },
            ActionError::ConfirmationRequired,
            ActionError::ConfirmationExpired,
            ActionError::ConfirmationMismatch,
            ActionError::ApplicationNotAllowed {
                application_id: "app_1".to_string(),
            },
            ActionError::ExecutableChanged {
                application_id: "app_1".to_string(),
            },
            ActionError::WindowNotFound,
            ActionError::WindowExpired,
            ActionError::TimerNotFound,
            ActionError::ScreenshotFailed {
                detail: "no monitor".to_string(),
            },
            ActionError::SensitiveWindow,
            ActionError::WindowsApiError { code: 5 },
            ActionError::StorageError,
            ActionError::Busy,
            ActionError::Cancelled,
        ];
        let mut codes: Vec<&str> = samples.iter().map(ActionError::code).collect();
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(
            codes.len(),
            samples.len(),
            "every variant needs its own code"
        );
        for sample in &samples {
            assert!(!sample.to_string().is_empty());
        }
    }

    #[test]
    fn a_platform_error_never_carries_a_system_message() {
        // Windows returns a localized, unbounded message; only the code is kept.
        let error = ActionError::WindowsApiError { code: 5 };
        let rendered = error.to_string();
        assert!(rendered.contains("5"));
        assert!(rendered.contains("code"));
        assert!(!rendered.contains("Access is denied"));
    }

    #[test]
    fn errors_are_classified_for_the_interface() {
        assert!(ActionError::ForbiddenAction {
            reason: "shell".to_string()
        }
        .is_policy_refusal());
        assert!(!ActionError::Cancelled.is_policy_refusal());
        assert!(ActionError::ConfirmationExpired.is_confirmation_related());
        assert!(!ActionError::WindowNotFound.is_confirmation_related());
    }

    #[test]
    fn an_io_failure_becomes_a_content_free_storage_error() {
        let io = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "C:/secret/path");
        let error: ActionError = io.into();
        assert_eq!(error, ActionError::StorageError);
        assert!(!error.to_string().contains("C:/secret/path"));
    }
}
