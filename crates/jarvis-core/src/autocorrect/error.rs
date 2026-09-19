//! Errors of the local autocorrect feature.
//!
//! Every variant carries a content-free message. A spelling issue names a word the
//! user typed, but an *error* never carries document text, an AI answer, or a
//! detected secret: the interface can log and show these messages safely.

use std::fmt;

use crate::memory::SecretKind;
use crate::sync::SyncError;

/// Everything that can go wrong while checking, correcting, or improving text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AutocorrectError {
    /// No dictionary file was found for this language.
    DictionaryMissing {
        language: String,
        /// Paths the interface can show so the user knows what to install.
        expected: Vec<String>,
    },
    /// A dictionary file exists but could not be parsed.
    DictionaryInvalid { language: String, reason: String },
    /// The master key is not in memory, so the user dictionary cannot be read.
    StorageLocked,
    /// The durable store refused the operation.
    Storage(SyncError),
    /// The entry does not exist.
    NotFound,
    /// The word is already in the user dictionary.
    Duplicate,
    /// The word is empty, too long, or otherwise not a word.
    InvalidWord,
    /// The user dictionary is full.
    DictionaryFull { limit: usize },
    /// The text is longer than one interactive check accepts.
    TextTooLarge { limit: usize },
    /// The text is empty, so there is nothing to check or improve.
    EmptyText,
    /// The text changed after it was checked, so the correction is refused.
    StaleText,
    /// A correction no longer matches the text it was computed for.
    RangeMismatch,
    /// Two corrections touch the same text.
    OverlappingCorrections,
    /// Nothing to undo, or the correction was already replaced by the user.
    UndoConflict,
    /// The local model is not running or not configured.
    AiUnavailable,
    /// The user cancelled the generation.
    Cancelled,
    /// The model answered something unusable.
    ModelOutput,
    /// The text looks like it carries a secret, so it was not sent to the model.
    SecretDetected(Vec<SecretKind>),
    /// The settings cannot be used as written.
    InvalidConfiguration,
    /// A file operation failed.
    Io,
}

impl AutocorrectError {
    /// Stable, content-free code for the interface.
    pub fn code(&self) -> &'static str {
        match self {
            Self::DictionaryMissing { .. } => "dictionary_missing",
            Self::DictionaryInvalid { .. } => "dictionary_invalid",
            Self::StorageLocked => "storage_locked",
            Self::Storage(_) => "storage_error",
            Self::NotFound => "not_found",
            Self::Duplicate => "duplicate",
            Self::InvalidWord => "invalid_word",
            Self::DictionaryFull { .. } => "dictionary_full",
            Self::TextTooLarge { .. } => "text_too_large",
            Self::EmptyText => "empty_text",
            Self::StaleText => "stale_text",
            Self::RangeMismatch => "range_mismatch",
            Self::OverlappingCorrections => "overlapping_corrections",
            Self::UndoConflict => "undo_conflict",
            Self::AiUnavailable => "ai_unavailable",
            Self::Cancelled => "cancelled",
            Self::ModelOutput => "model_output",
            Self::SecretDetected(_) => "secret_detected",
            Self::InvalidConfiguration => "invalid_configuration",
            Self::Io => "io_error",
        }
    }

    /// Whether the failure is the secret gate refusing to send text to the model.
    pub fn is_secret_related(&self) -> bool {
        matches!(self, Self::SecretDetected(_))
    }

    /// The secret kinds involved, when the failure is secret-related.
    pub fn secret_kinds(&self) -> &[SecretKind] {
        match self {
            Self::SecretDetected(kinds) => kinds,
            _ => &[],
        }
    }

    /// The dictionary paths the interface should show, when a dictionary is missing.
    pub fn expected_paths(&self) -> &[String] {
        match self {
            Self::DictionaryMissing { expected, .. } => expected,
            _ => &[],
        }
    }
}

impl fmt::Display for AutocorrectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DictionaryMissing { language, .. } => write!(
                formatter,
                "no {language} dictionary is installed; add the .aff and .dic files to the dictionaries folder"
            ),
            Self::DictionaryInvalid { language, reason } => {
                write!(formatter, "the {language} dictionary could not be read: {reason}")
            }
            Self::StorageLocked => {
                formatter.write_str("the encrypted storage is locked, so your dictionary is unavailable")
            }
            Self::Storage(error) => write!(formatter, "storage error: {error}"),
            Self::NotFound => formatter.write_str("the entry does not exist"),
            Self::Duplicate => formatter.write_str("that word is already in your dictionary"),
            Self::InvalidWord => formatter.write_str("that is not a word that can be added"),
            Self::DictionaryFull { limit } => {
                write!(formatter, "your dictionary is full ({limit} words)")
            }
            Self::TextTooLarge { limit } => write!(
                formatter,
                "the text is too long for one check ({limit} characters)"
            ),
            Self::EmptyText => formatter.write_str("there is no text to work with"),
            Self::StaleText => {
                formatter.write_str("the text changed since it was checked; check it again")
            }
            Self::RangeMismatch => {
                formatter.write_str("the correction no longer matches this text")
            }
            Self::OverlappingCorrections => {
                formatter.write_str("two selected corrections touch the same text")
            }
            Self::UndoConflict => formatter.write_str(
                "the corrected text was edited afterwards, so it cannot be undone automatically",
            ),
            Self::AiUnavailable => {
                formatter.write_str("the local model is not running, so the text was not sent")
            }
            Self::Cancelled => formatter.write_str("the generation was cancelled"),
            Self::ModelOutput => formatter.write_str("the model did not return usable text"),
            Self::SecretDetected(kinds) => {
                let described: Vec<&str> = kinds.iter().map(SecretKind::describe).collect();
                write!(
                    formatter,
                    "this text looks like it contains {}; it was not sent to the model",
                    if described.is_empty() {
                        "sensitive data".to_string()
                    } else {
                        described.join(", ")
                    }
                )
            }
            Self::InvalidConfiguration => formatter.write_str("the settings are not usable"),
            Self::Io => formatter.write_str("a file operation failed"),
        }
    }
}

impl std::error::Error for AutocorrectError {}

impl From<SyncError> for AutocorrectError {
    fn from(error: SyncError) -> Self {
        Self::Storage(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_has_a_stable_code() {
        let samples = [
            AutocorrectError::DictionaryMissing {
                language: "english".to_string(),
                expected: vec!["en_US.aff".to_string()],
            },
            AutocorrectError::DictionaryInvalid {
                language: "russian".to_string(),
                reason: "truncated".to_string(),
            },
            AutocorrectError::StorageLocked,
            AutocorrectError::Storage(SyncError::StorageBusy),
            AutocorrectError::NotFound,
            AutocorrectError::Duplicate,
            AutocorrectError::InvalidWord,
            AutocorrectError::DictionaryFull { limit: 10 },
            AutocorrectError::TextTooLarge { limit: 10 },
            AutocorrectError::EmptyText,
            AutocorrectError::StaleText,
            AutocorrectError::RangeMismatch,
            AutocorrectError::OverlappingCorrections,
            AutocorrectError::UndoConflict,
            AutocorrectError::AiUnavailable,
            AutocorrectError::Cancelled,
            AutocorrectError::ModelOutput,
            AutocorrectError::SecretDetected(vec![SecretKind::ApiToken]),
            AutocorrectError::InvalidConfiguration,
            AutocorrectError::Io,
        ];
        let mut codes: Vec<&str> = samples.iter().map(AutocorrectError::code).collect();
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
    fn a_missing_dictionary_lists_the_paths_the_user_must_install() {
        let error = AutocorrectError::DictionaryMissing {
            language: "english".to_string(),
            expected: vec![
                "C:/data/dictionaries/en_US.aff".to_string(),
                "C:/data/dictionaries/en_US.dic".to_string(),
            ],
        };
        assert_eq!(error.code(), "dictionary_missing");
        assert_eq!(error.expected_paths().len(), 2);
        assert!(error
            .expected_paths()
            .iter()
            .any(|path| path.ends_with("en_US.dic")));
        // The message names the language; the exact paths travel in `expected_paths`,
        // which the interface shows next to an "open folder" action.
        assert!(error.to_string().contains("english"));
        assert!(error.to_string().contains("dictionaries folder"));
        assert!(error.secret_kinds().is_empty());
    }

    #[test]
    fn a_secret_error_names_kinds_and_never_the_value() {
        let error =
            AutocorrectError::SecretDetected(vec![SecretKind::PrivateKey, SecretKind::PaymentCard]);
        let rendered = error.to_string();
        assert!(rendered.contains("private key"));
        assert!(rendered.contains("payment card"));
        assert!(rendered.contains("not sent"));
        assert!(error.is_secret_related());
        assert_eq!(error.secret_kinds().len(), 2);
        // No matched text could appear: the error only holds kinds.
        assert!(!rendered.contains("FICTIONAL"));
    }
}
