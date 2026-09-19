//! Errors of the encrypted AI memory.
//!
//! Every variant carries a content-free message: nothing in this module ever
//! renders a conversation, a summary, a fact, or a detected secret, so an error
//! can be logged and shown in the interface without leaking stored memory.
//!
//! The secret variants carry the *kinds* that were found, never the text. That is
//! what lets the interface warn the user ("looks like a private key") while the
//! value itself stays out of logs, dialogs, and telemetry.

use std::fmt;

use crate::sync::SyncError;

/// What a heuristic detector recognized. Ordering is stable so the kind list can
/// be compared in tests.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SecretKind {
    /// A PEM or OpenSSH private-key block.
    PrivateKey,
    /// A recognizable provider token (`sk-…`, `ghp_…`, `AKIA…`, `xoxb-…`).
    ApiToken,
    /// A JSON Web Token.
    Jwt,
    /// A one-time recovery code, usually a group of separated blocks.
    RecoveryCode,
    /// A line that assigns something to a password-like name.
    PasswordAssignment,
    /// A long, high-entropy token without a known prefix.
    HighEntropyToken,
    /// A digit sequence that satisfies the Luhn check.
    PaymentCard,
}

impl SecretKind {
    /// Stable, content-free label used in messages and tests.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrivateKey => "private_key",
            Self::ApiToken => "api_token",
            Self::Jwt => "jwt",
            Self::RecoveryCode => "recovery_code",
            Self::PasswordAssignment => "password_assignment",
            Self::HighEntropyToken => "high_entropy_token",
            Self::PaymentCard => "payment_card",
        }
    }

    /// Human-readable, content-free description for the interface.
    pub fn describe(&self) -> &'static str {
        match self {
            Self::PrivateKey => "a private key",
            Self::ApiToken => "an API token",
            Self::Jwt => "a JSON Web Token",
            Self::RecoveryCode => "a recovery code",
            Self::PasswordAssignment => "an assigned password",
            Self::HighEntropyToken => "a long random-looking token",
            Self::PaymentCard => "a payment card number",
        }
    }

    pub fn all() -> [Self; 7] {
        [
            Self::PrivateKey,
            Self::ApiToken,
            Self::Jwt,
            Self::RecoveryCode,
            Self::PasswordAssignment,
            Self::HighEntropyToken,
            Self::PaymentCard,
        ]
    }
}

/// Everything that can go wrong while storing or building memory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryError {
    /// The master key is not in memory, so no derived memory key exists.
    StorageLocked,
    /// The user switched the memory feature off.
    MemoryDisabled,
    NotFound,
    /// The entity carries a tombstone.
    Deleted,
    /// The record exists but this key cannot read it.
    Unreadable,
    /// The payload was written by a newer version of the application.
    UnsupportedPayloadVersion,
    /// The payload is not what this version expects.
    MalformedPayload,
    /// Text, or the number of items, exceeds the documented limit.
    ContentTooLarge,
    InvalidTimestamp,
    /// The text was refused because it looks like it contains a secret.
    SecretDetected(Vec<SecretKind>),
    /// A manual save was asked for without accepting the secret warning.
    SecretConfirmationRequired(Vec<SecretKind>),
    /// A destructive operation was asked for without an explicit confirmation.
    ConfirmationRequired,
    /// A summarization job is already running for this conversation.
    SummaryInProgress,
    /// No model is available to summarize with.
    SummaryUnavailable,
    /// The model answered something that is not a usable summary.
    SummarizerOutput,
    /// The configuration cannot be used as written.
    InvalidConfiguration,
    /// The durable store refused the operation.
    Storage(SyncError),
    /// A file operation failed.
    Io,
}

impl MemoryError {
    /// Whether the failure is caused by the secret filter.
    pub fn is_secret_related(&self) -> bool {
        matches!(
            self,
            Self::SecretDetected(_) | Self::SecretConfirmationRequired(_)
        )
    }

    /// The secret kinds involved, when the failure is secret-related.
    pub fn secret_kinds(&self) -> &[SecretKind] {
        match self {
            Self::SecretDetected(kinds) | Self::SecretConfirmationRequired(kinds) => kinds,
            _ => &[],
        }
    }

    /// Stable, content-free code for the interface.
    pub fn code(&self) -> &'static str {
        match self {
            Self::StorageLocked => "storage_locked",
            Self::MemoryDisabled => "memory_disabled",
            Self::NotFound => "not_found",
            Self::Deleted => "deleted",
            Self::Unreadable => "unreadable",
            Self::UnsupportedPayloadVersion => "unsupported_payload_version",
            Self::MalformedPayload => "malformed_payload",
            Self::ContentTooLarge => "content_too_large",
            Self::InvalidTimestamp => "invalid_timestamp",
            Self::SecretDetected(_) => "secret_detected",
            Self::SecretConfirmationRequired(_) => "secret_confirmation_required",
            Self::ConfirmationRequired => "confirmation_required",
            Self::SummaryInProgress => "summary_in_progress",
            Self::SummaryUnavailable => "summary_unavailable",
            Self::SummarizerOutput => "summarizer_output",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::Storage(_) => "storage_error",
            Self::Io => "io_error",
        }
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StorageLocked => formatter.write_str("the encrypted storage is locked"),
            Self::MemoryDisabled => formatter.write_str("AI memory is turned off"),
            Self::NotFound => formatter.write_str("the entry does not exist"),
            Self::Deleted => formatter.write_str("the entry was deleted"),
            Self::Unreadable => formatter.write_str("the entry could not be decrypted"),
            Self::UnsupportedPayloadVersion => {
                formatter.write_str("the entry was written by a newer version")
            }
            Self::MalformedPayload => formatter.write_str("the stored entry is malformed"),
            Self::ContentTooLarge => formatter.write_str("the text is too long to store"),
            Self::InvalidTimestamp => formatter.write_str("the text has an invalid timestamp"),
            Self::SecretDetected(kinds) => write!(
                formatter,
                "this looks like it contains {} and was not saved automatically",
                describe_kinds(kinds)
            ),
            Self::SecretConfirmationRequired(kinds) => write!(
                formatter,
                "this looks like it contains {}; confirm to save it anyway",
                describe_kinds(kinds)
            ),
            Self::ConfirmationRequired => {
                formatter.write_str("this needs an explicit confirmation first")
            }
            Self::SummaryInProgress => {
                formatter.write_str("a summary for this conversation is already running")
            }
            Self::SummaryUnavailable => {
                formatter.write_str("no model is available to write a summary")
            }
            Self::SummarizerOutput => {
                formatter.write_str("the model did not return a usable summary")
            }
            Self::InvalidConfiguration => formatter.write_str("the settings are not usable"),
            Self::Storage(error) => write!(formatter, "storage error: {error}"),
            Self::Io => formatter.write_str("a file operation failed"),
        }
    }
}

impl std::error::Error for MemoryError {}

impl From<SyncError> for MemoryError {
    fn from(error: SyncError) -> Self {
        Self::Storage(error)
    }
}

/// Joins kind descriptions into one readable clause, without any secret text.
fn describe_kinds(kinds: &[SecretKind]) -> String {
    if kinds.is_empty() {
        return "sensitive data".to_string();
    }
    kinds
        .iter()
        .map(SecretKind::describe)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_stable_label_and_description() {
        let kinds = SecretKind::all();
        assert_eq!(kinds.len(), 7);
        let mut labels: Vec<&str> = kinds.iter().map(SecretKind::as_str).collect();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), kinds.len(), "labels must be unique");
        for kind in kinds {
            assert!(!kind.describe().is_empty());
            assert!(!kind.as_str().contains(char::is_whitespace));
        }
    }

    #[test]
    fn a_secret_error_never_contains_the_secret() {
        let error = MemoryError::SecretDetected(vec![SecretKind::PrivateKey, SecretKind::Jwt]);
        let rendered = format!("{error}");
        assert!(rendered.contains("private key"));
        assert!(rendered.contains("JSON Web Token"));
        // Nothing secret-shaped survived, only the kind descriptions.
        assert!(!rendered.contains("BEGIN"));
        assert_eq!(error.code(), "secret_detected");
        assert!(error.is_secret_related());
        assert_eq!(error.secret_kinds().len(), 2);
    }

    #[test]
    fn confirmation_required_is_distinguishable_from_blocked() {
        let blocked = MemoryError::SecretDetected(vec![SecretKind::ApiToken]);
        let needs_confirmation =
            MemoryError::SecretConfirmationRequired(vec![SecretKind::ApiToken]);
        assert_ne!(blocked.code(), needs_confirmation.code());
        assert!(needs_confirmation.is_secret_related());
        assert!(format!("{needs_confirmation}").contains("confirm"));
    }

    #[test]
    fn storage_errors_keep_their_cause_and_stay_content_free() {
        let error = MemoryError::from(SyncError::StorageBusy);
        assert_eq!(error.code(), "storage_error");
        assert!(format!("{error}").contains("busy") || format!("{error}").contains("storage"));
        assert!(!error.is_secret_related());
        assert!(error.secret_kinds().is_empty());
    }
}
