//! Settings and validation for local Whisper dictation.
//!
//! Everything here is a path the user picked, a small number, or a switch, so
//! the document is readable while the encrypted storages are locked — exactly
//! when a user decides whether dictation should run at all. No key, no
//! password, and no transcript is ever part of it.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::error::WhisperError;

/// File name of the settings document inside the feature directory.
pub const SETTINGS_FILE: &str = "whisper-settings.json";
/// Schema version of the settings document.
pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

/// Sample rate the recorder and Whisper both use.
pub const SAMPLE_RATE: u32 = 16_000;
/// Longest recording a single dictation may produce, in seconds.
pub const MAX_DICTATION_SECONDS: u64 = 300;
/// Shortest recording worth sending to the model, in seconds.
pub const MIN_DICTATION_SECONDS: u64 = 1;
/// Shortest audio that is still sent to the model, in milliseconds.
///
/// This is the floor for "something was recorded", not a preference: a
/// recording that ended on silence after a short word is still transcribed, and
/// only a buffer that is effectively empty is refused.
pub const MIN_AUDIO_MS: u64 = 200;
/// Default recording length when the user does not stop it earlier.
pub const DEFAULT_DICTATION_SECONDS: u64 = 30;
/// Longest a transcription may take before the process is stopped.
pub const MIN_TIMEOUT_SECONDS: u64 = 5;
pub const MAX_TIMEOUT_SECONDS: u64 = 600;
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 120;
/// Threads offered to the model.
pub const MIN_THREADS: u8 = 1;
pub const MAX_THREADS: u8 = 32;
pub const DEFAULT_THREADS: u8 = 4;
/// Silence that ends a dictation early, in milliseconds.
pub const MIN_SILENCE_MS: u64 = 500;
pub const MAX_SILENCE_MS: u64 = 10_000;
pub const DEFAULT_SILENCE_MS: u64 = 1_500;
/// Loudness below which a frame counts as silence, on the 16-bit scale.
pub const DEFAULT_SILENCE_PEAK: i16 = 400;

/// Languages this build offers, as Whisper spells them.
pub const LANGUAGES: [&str; 7] = ["auto", "ru", "en", "ua", "de", "fr", "es"];

/// What the user may configure.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WhisperSettings {
    /// Whether dictation is allowed at all. Off by default: a microphone is
    /// never opened because an application was installed.
    pub enabled: bool,
    /// The `whisper-cli.exe` (or `main.exe`) the user picked.
    pub binary_path: String,
    /// The `ggml-*.bin` model the user picked.
    pub model_path: String,
    /// Language hint, or `auto`.
    pub language: String,
    /// Whether to translate into English instead of transcribing.
    pub translate: bool,
    /// Threads handed to the model.
    pub threads: u8,
    /// How long one recording may be.
    pub max_seconds: u64,
    /// Silence that ends a recording early.
    pub silence_ms: u64,
    /// How long the process may run.
    pub timeout_seconds: u64,
    /// Whether to keep the temporary WAV after a transcription. Off by default:
    /// the audio is deleted as soon as the text exists.
    pub keep_audio: bool,
    /// Whether the window may start a dictation from its own button. The tray
    /// item is controlled by the same switch.
    pub allow_from_window: bool,
    pub schema_version: u32,
}

impl Default for WhisperSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            binary_path: String::new(),
            model_path: String::new(),
            language: "auto".to_string(),
            translate: false,
            threads: DEFAULT_THREADS,
            max_seconds: DEFAULT_DICTATION_SECONDS,
            silence_ms: DEFAULT_SILENCE_MS,
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            keep_audio: false,
            allow_from_window: true,
            schema_version: SETTINGS_SCHEMA_VERSION,
        }
    }
}

impl WhisperSettings {
    /// Repairs out-of-range values instead of refusing to load.
    pub fn normalized(mut self) -> Self {
        self.threads = self.threads.clamp(MIN_THREADS, MAX_THREADS);
        self.max_seconds = self
            .max_seconds
            .clamp(MIN_DICTATION_SECONDS, MAX_DICTATION_SECONDS);
        self.silence_ms = self.silence_ms.clamp(MIN_SILENCE_MS, MAX_SILENCE_MS);
        self.timeout_seconds = self
            .timeout_seconds
            .clamp(MIN_TIMEOUT_SECONDS, MAX_TIMEOUT_SECONDS);
        if !LANGUAGES.contains(&self.language.as_str()) {
            self.language = "auto".to_string();
        }
        self.binary_path = self.binary_path.trim().to_string();
        self.model_path = self.model_path.trim().to_string();
        self.schema_version = SETTINGS_SCHEMA_VERSION;
        self
    }

    /// Checks everything a transcription needs before anything is started.
    ///
    /// A path that is set but unusable is an error; a path that is empty is
    /// only `NotConfigured`, because skipping dictation is a valid choice.
    pub fn validate(&self) -> Result<(), WhisperError> {
        if !LANGUAGES.contains(&self.language.as_str()) {
            return Err(WhisperError::UnsupportedLanguage(self.language.clone()));
        }
        if !(MIN_THREADS..=MAX_THREADS).contains(&self.threads) {
            return Err(WhisperError::InvalidConfiguration(format!(
                "threads must be {MIN_THREADS} to {MAX_THREADS}"
            )));
        }
        if !(MIN_DICTATION_SECONDS..=MAX_DICTATION_SECONDS).contains(&self.max_seconds) {
            return Err(WhisperError::InvalidConfiguration(format!(
                "a recording is {MIN_DICTATION_SECONDS} to {MAX_DICTATION_SECONDS} seconds long"
            )));
        }
        if !(MIN_SILENCE_MS..=MAX_SILENCE_MS).contains(&self.silence_ms) {
            return Err(WhisperError::InvalidConfiguration(format!(
                "silence is {MIN_SILENCE_MS} to {MAX_SILENCE_MS} milliseconds"
            )));
        }
        if !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&self.timeout_seconds) {
            return Err(WhisperError::InvalidConfiguration(format!(
                "the timeout must be {MIN_TIMEOUT_SECONDS} to {MAX_TIMEOUT_SECONDS} seconds"
            )));
        }
        if self.binary_path.is_empty() && self.model_path.is_empty() {
            return Err(WhisperError::NotConfigured);
        }
        if self.binary_path.is_empty() {
            return Err(WhisperError::BinaryUnavailable(
                "no executable was chosen".to_string(),
            ));
        }
        if self.model_path.is_empty() {
            return Err(WhisperError::ModelUnavailable(
                "no model was chosen".to_string(),
            ));
        }
        Ok(())
    }

    /// Whether both files are configured, which is what the interface shows.
    pub fn is_configured(&self) -> bool {
        !self.binary_path.is_empty() && !self.model_path.is_empty()
    }

    /// Whether a missing path is simply "not chosen yet".
    pub fn not_configured(&self) -> bool {
        self.binary_path.is_empty() || self.model_path.is_empty()
    }

    pub fn to_json(&self) -> Result<String, WhisperError> {
        serde_json::to_string(self).map_err(|_| WhisperError::Storage)
    }

    pub fn from_json(text: &str) -> Result<Self, WhisperError> {
        serde_json::from_str(text).map_err(|_| {
            WhisperError::InvalidConfiguration("the settings document is damaged".to_string())
        })
    }

    /// Reads the stored document, falling back to the defaults.
    ///
    /// A damaged document is not an error the user has to fix: the safe
    /// defaults (dictation off, no paths) come back instead.
    pub fn load_or_default(text: Option<&str>) -> Self {
        text.and_then(|text| Self::from_json(text).ok())
            .unwrap_or_default()
            .normalized()
    }
}

/// The settings document inside a feature directory, if it is readable.
pub fn stored_settings(directory: &Path) -> Option<WhisperSettings> {
    let text = std::fs::read_to_string(directory.join(SETTINGS_FILE)).ok()?;
    WhisperSettings::from_json(&text).ok()
}

/// A content-free description of what is stored, for the diagnostics report.
///
/// It answers the question a person actually has after choosing two files —
/// "did it save?" — without putting a path into a report that may be copied
/// somewhere else. Only the *presence* and the *shape* of the values are
/// described; the executable's name and the model's name are safe to show, and
/// a full path is not.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSettingsSummary {
    /// Whether the document exists at all.
    pub document_present: bool,
    /// Whether it could be parsed. A damaged document falls back to the defaults.
    pub document_readable: bool,
    pub enabled: bool,
    /// Whether an executable is set, and its file name only.
    pub executable_set: bool,
    pub executable_name: String,
    pub model_set: bool,
    pub model_name: String,
    pub language: String,
    /// Whether the stored document names both files, which is what "configured"
    /// means for the rest of the application.
    pub configured: bool,
    /// A version mismatch means the document came from another build.
    pub schema_version: u32,
}

impl StoredSettingsSummary {
    /// Reads the document and describes it without revealing a path.
    pub fn read(directory: &Path) -> Self {
        let path = directory.join(SETTINGS_FILE);
        let text = std::fs::read_to_string(&path).ok();
        let document_present = text.is_some();
        let settings = text
            .as_deref()
            .and_then(|text| WhisperSettings::from_json(text).ok());
        let document_readable = settings.is_some();
        let settings = settings.unwrap_or_default();
        Self {
            document_present,
            document_readable,
            enabled: settings.enabled,
            executable_set: !settings.binary_path.trim().is_empty(),
            executable_name: if settings.binary_path.trim().is_empty() {
                String::new()
            } else {
                crate::text::file_label(&settings.binary_path)
            },
            model_set: !settings.model_path.trim().is_empty(),
            model_name: if settings.model_path.trim().is_empty() {
                String::new()
            } else {
                crate::text::file_label(&settings.model_path)
            },
            language: settings.language.clone(),
            configured: settings.is_configured(),
            schema_version: settings.schema_version,
        }
    }

    /// One line per fact, with no path anywhere.
    pub fn describe(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "dictation settings document: {}",
            if !self.document_present {
                "absent"
            } else if self.document_readable {
                "readable"
            } else {
                "present but damaged, so the defaults are in use"
            }
        ));
        lines.push(format!("dictation enabled: {}", self.enabled));
        lines.push(format!(
            "executable set: {}{}",
            self.executable_set,
            if self.executable_name.is_empty() {
                String::new()
            } else {
                format!(" ({})", self.executable_name)
            }
        ));
        lines.push(format!(
            "model set: {}{}",
            self.model_set,
            if self.model_name.is_empty() {
                String::new()
            } else {
                format!(" ({})", self.model_name)
            }
        ));
        lines.push(format!("language: {}", self.language));
        lines.push(format!("configured: {}", self.configured));
        lines.push(format!("settings schema version: {}", self.schema_version));
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_never_open_a_microphone() {
        let settings = WhisperSettings::default();
        assert!(!settings.enabled, "dictation must be off until asked for");
        assert_eq!(settings.language, "auto");
        assert_eq!(settings.threads, DEFAULT_THREADS);
        assert!(!settings.keep_audio, "audio is not kept unless asked for");
        assert_eq!(settings.schema_version, SETTINGS_SCHEMA_VERSION);
        assert!(settings.not_configured());
    }

    #[test]
    fn an_empty_configuration_is_not_a_broken_one() {
        // Skipping dictation is a choice, not a fault: it is `NotConfigured`.
        let settings = WhisperSettings::default();
        assert_eq!(
            settings.validate().unwrap_err(),
            WhisperError::NotConfigured
        );
        assert_eq!(settings.validate().unwrap_err().code(), "not_configured");
    }

    #[test]
    fn half_a_configuration_names_the_half_that_is_missing() {
        let settings = WhisperSettings {
            binary_path: "C:/whisper/whisper-cli.exe".to_string(),
            ..WhisperSettings::default()
        };
        assert!(matches!(
            settings.validate().unwrap_err(),
            WhisperError::ModelUnavailable(_)
        ));
        let settings = WhisperSettings {
            model_path: "C:/models/ggml-small.bin".to_string(),
            ..WhisperSettings::default()
        };
        assert!(matches!(
            settings.validate().unwrap_err(),
            WhisperError::BinaryUnavailable(_)
        ));
    }

    #[test]
    fn damaged_values_are_repaired_instead_of_refusing_to_load() {
        let damaged = WhisperSettings {
            enabled: true,
            model_path: "C:/models/ggml-small.bin".to_string(),
            threads: 200,
            max_seconds: 1_000_000,
            silence_ms: 5,
            timeout_seconds: 0,
            language: "xx".to_string(),
            binary_path: "  C:/whisper/whisper-cli.exe  ".to_string(),
            ..WhisperSettings::default()
        };
        let repaired = damaged.normalized();
        assert_eq!(repaired.threads, MAX_THREADS);
        assert_eq!(repaired.max_seconds, MAX_DICTATION_SECONDS);
        assert_eq!(repaired.silence_ms, MIN_SILENCE_MS);
        assert_eq!(repaired.timeout_seconds, MIN_TIMEOUT_SECONDS);
        assert_eq!(repaired.language, "auto");
        assert_eq!(repaired.binary_path, "C:/whisper/whisper-cli.exe");
        assert_eq!(repaired.validate(), Ok(()));
    }

    #[test]
    fn the_document_round_trips_and_a_damaged_one_falls_back_to_the_defaults() {
        let settings = WhisperSettings {
            enabled: true,
            binary_path: "C:/whisper/whisper-cli.exe".to_string(),
            model_path: "C:/models/ggml-small.bin".to_string(),
            language: "ru".to_string(),
            ..WhisperSettings::default()
        };
        let text = settings.to_json().unwrap();
        assert_eq!(WhisperSettings::from_json(&text).unwrap(), settings);
        assert_eq!(
            WhisperSettings::load_or_default(Some("{not json")),
            WhisperSettings::default()
        );
        assert_eq!(
            WhisperSettings::load_or_default(None),
            WhisperSettings::default()
        );
        assert!(WhisperSettings::from_json("{not json")
            .unwrap_err()
            .code()
            .eq("invalid_configuration"));
    }

    #[test]
    fn a_language_outside_the_offered_set_is_refused_by_validation() {
        let settings = WhisperSettings {
            language: "klingon".to_string(),
            binary_path: "a".to_string(),
            model_path: "b".to_string(),
            ..WhisperSettings::default()
        };
        assert!(matches!(
            settings.validate().unwrap_err(),
            WhisperError::UnsupportedLanguage(_)
        ));
    }

    #[test]
    fn the_ceiling_on_a_recording_is_the_one_the_configuration_uses() {
        // These are the numbers the interface shows, so a change here is a change
        // a person can see: the constants are pinned on purpose.
        const _: () = assert!(MAX_DICTATION_SECONDS == 300);
        const _: () = assert!(SAMPLE_RATE == 16_000);
        const _: () = assert!(MIN_DICTATION_SECONDS < DEFAULT_DICTATION_SECONDS);
        const _: () = assert!(DEFAULT_DICTATION_SECONDS < MAX_DICTATION_SECONDS);
    }
}
