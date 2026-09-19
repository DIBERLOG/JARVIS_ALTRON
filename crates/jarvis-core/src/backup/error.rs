//! What can go wrong while making or opening a backup.
//!
//! Every variant is a stable code plus a short sentence. None of them carries a
//! path, a password, a key, a note name, or any content: a backup error is shown
//! in the window, written to the log, and put in the diagnostics report, and all
//! three of those are places a secret must never reach.

use std::fmt;

use crate::sync::crypto::CryptoError;

/// Why a backup or a restore was refused.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackupError {
    /// The password did not unwrap the container's key envelope.
    WrongPasswordOrDamaged,
    /// The file does not start with the container magic.
    NotAContainer,
    /// The container declares a version this build does not write.
    UnsupportedVersion,
    /// A header field is missing, malformed, or outside its bounds.
    InvalidHeader(String),
    /// The manifest hash does not match the manifest it claims to describe.
    ManifestMismatch,
    /// An entry's decrypted bytes do not have the SHA-256 the manifest states.
    ContentMismatch,
    /// An entry is missing from the container, or the container has extra bytes.
    TruncatedContainer,
    /// The container declares more entries, or larger ones, than this build allows.
    TooLarge,
    /// An entry name is not a safe logical name.
    UnsafeEntryName,
    /// The same logical name appears twice.
    DuplicateEntryName,
    /// A SQLite file did not pass `PRAGMA integrity_check`.
    IntegrityCheckFailed,
    /// The container was written by a schema this build is too old to open.
    SchemaTooNew,
    /// The chosen destination cannot be written.
    DestinationUnavailable,
    /// The password was shorter than the minimum.
    PasswordTooShort,
    /// A component could not be read or written on disk.
    Storage,
    /// The restore journal says a restore was interrupted and could not be
    /// completed or rolled back.
    RecoveryFailed,
    /// The local key could not be re-bound to this machine (DPAPI).
    LocalKeyFailed,
    /// Another backup or restore is already running.
    Busy,
}

impl BackupError {
    /// A stable, content-free code for the interface and the log.
    pub fn code(&self) -> &'static str {
        match self {
            Self::WrongPasswordOrDamaged => "wrong_password_or_damaged",
            Self::NotAContainer => "not_a_container",
            Self::UnsupportedVersion => "unsupported_version",
            Self::InvalidHeader(_) => "invalid_header",
            Self::ManifestMismatch => "manifest_mismatch",
            Self::ContentMismatch => "content_mismatch",
            Self::TruncatedContainer => "truncated_container",
            Self::TooLarge => "too_large",
            Self::UnsafeEntryName => "unsafe_entry_name",
            Self::DuplicateEntryName => "duplicate_entry_name",
            Self::IntegrityCheckFailed => "integrity_check_failed",
            Self::SchemaTooNew => "schema_too_new",
            Self::DestinationUnavailable => "destination_unavailable",
            Self::PasswordTooShort => "password_too_short",
            Self::Storage => "storage",
            Self::RecoveryFailed => "recovery_failed",
            Self::LocalKeyFailed => "local_key_failed",
            Self::Busy => "busy",
        }
    }

    /// A short, content-free detail, when the variant carries one.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::InvalidHeader(detail) => Some(detail.clone()),
            _ => None,
        }
    }
}

impl fmt::Display for BackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongPasswordOrDamaged => {
                formatter.write_str("the password is wrong or the backup is damaged")
            }
            Self::NotAContainer => formatter.write_str("that file is not a JARVIS backup"),
            Self::UnsupportedVersion => {
                formatter.write_str("that backup was written by another version of the format")
            }
            Self::InvalidHeader(detail) => {
                write!(formatter, "the backup header is not valid: {detail}")
            }
            Self::ManifestMismatch => {
                formatter.write_str("the backup manifest does not match its contents")
            }
            Self::ContentMismatch => {
                formatter.write_str("a part of the backup does not match its checksum")
            }
            Self::TruncatedContainer => {
                formatter.write_str("the backup file ends before it should")
            }
            Self::TooLarge => formatter.write_str("the backup is larger than this build accepts"),
            Self::UnsafeEntryName => {
                formatter.write_str("the backup contains an unsafe entry name")
            }
            Self::DuplicateEntryName => {
                formatter.write_str("the backup names the same entry twice")
            }
            Self::IntegrityCheckFailed => {
                formatter.write_str("a restored database did not pass its integrity check")
            }
            Self::SchemaTooNew => {
                formatter.write_str("the backup comes from a newer storage schema")
            }
            Self::DestinationUnavailable => formatter.write_str("that file cannot be written"),
            Self::PasswordTooShort => formatter.write_str("the master password is too short"),
            Self::Storage => {
                formatter.write_str("a file of the backup could not be read or written")
            }
            Self::RecoveryFailed => {
                formatter.write_str("an interrupted restore could not be completed automatically")
            }
            Self::LocalKeyFailed => {
                formatter.write_str("the restored key could not be bound to this Windows account")
            }
            Self::Busy => formatter.write_str("another backup or restore is already running"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<CryptoError> for BackupError {
    fn from(_: CryptoError) -> Self {
        // A crypto failure is either a wrong password or a damaged container, and
        // the distinction is deliberately not made: telling them apart would let
        // someone confirm a password guess with a corrupted file.
        Self::WrongPasswordOrDamaged
    }
}

impl From<std::io::Error> for BackupError {
    fn from(_: std::io::Error) -> Self {
        // The io error can carry a path, and a path is not kept.
        Self::Storage
    }
}

impl From<serde_json::Error> for BackupError {
    fn from(_: serde_json::Error) -> Self {
        Self::InvalidHeader("the header is not readable JSON".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<BackupError> {
        vec![
            BackupError::WrongPasswordOrDamaged,
            BackupError::NotAContainer,
            BackupError::UnsupportedVersion,
            BackupError::InvalidHeader("a field".to_string()),
            BackupError::ManifestMismatch,
            BackupError::ContentMismatch,
            BackupError::TruncatedContainer,
            BackupError::TooLarge,
            BackupError::UnsafeEntryName,
            BackupError::DuplicateEntryName,
            BackupError::IntegrityCheckFailed,
            BackupError::SchemaTooNew,
            BackupError::DestinationUnavailable,
            BackupError::PasswordTooShort,
            BackupError::Storage,
            BackupError::RecoveryFailed,
            BackupError::LocalKeyFailed,
            BackupError::Busy,
        ]
    }

    #[test]
    fn every_variant_has_its_own_code() {
        let mut codes: Vec<&'static str> = every_variant().iter().map(BackupError::code).collect();
        codes.sort_unstable();
        let unique = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), unique, "two variants share a code");
    }

    #[test]
    fn no_message_carries_a_path_or_a_secret() {
        for error in every_variant() {
            let rendered = error.to_string();
            for forbidden in ["FICTIONAL", "\\", "/", "password:", "key="] {
                assert!(
                    !rendered.contains(forbidden),
                    "{forbidden} must not appear in {rendered}"
                );
            }
            assert!(!error.code().contains('/'), "{}", error.code());
        }
    }

    #[test]
    fn a_crypto_failure_never_says_which_of_the_two_it_was() {
        let error = BackupError::from(CryptoError::InvalidPasswordOrCorruptData);
        assert_eq!(error, BackupError::WrongPasswordOrDamaged);
        assert_eq!(
            BackupError::from(CryptoError::UnsupportedFormat),
            BackupError::WrongPasswordOrDamaged
        );
        // And the detail is never a raw error string.
        assert!(error.detail().is_none());
    }
}
