//! Local autocorrect settings.
//!
//! Like the AI-memory switches, these are not secret (on/off flags, limits, timeouts),
//! so they live in the application settings store and remain readable while the
//! encrypted storage is locked — which is exactly when a user decides whether the
//! checker should run at all.
//!
//! Every automatic behaviour is **off by default**:
//!
//! * checking notes and the chat draft is on, because it only marks text and never
//!   changes it;
//! * safe auto-correction is off, so nothing is rewritten without the user; when it is
//!   switched on it may only apply reasons with exactly one sensible answer;
//! * unknown words are never corrected automatically, whatever these settings say;
//! * the AI text-improvement switch is off until the user asks for it explicitly.

use serde::{Deserialize, Serialize};

use super::error::AutocorrectError;
use super::model::{Language, LanguageMode, MAX_CUSTOM_RULES, MAX_DICTIONARY_WORD_CHARS};

/// Settings key used by the existing application settings store (`app.db`).
pub const SETTINGS_KEY: &str = "autocorrect_settings";
/// Schema version of the settings document.
pub const AUTOCORRECT_SETTINGS_SCHEMA_VERSION: u32 = 1;

/// Default time the checker may spend on one text, in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 3_000;
/// Default pause after the last keystroke before a background check runs.
pub const DEFAULT_DEBOUNCE_MS: u64 = 500;
/// Default number of suggestions offered per issue.
pub const DEFAULT_MAX_SUGGESTIONS: usize = 5;
/// Default number of issues one check reports.
pub const DEFAULT_MAX_ISSUES: usize = 200;

/// Bounds enforced by [`AutocorrectSettings::normalized`].
pub const MIN_MAX_SUGGESTIONS: usize = 1;
pub const MAX_MAX_SUGGESTIONS: usize = 8;
pub const MIN_MAX_ISSUES: usize = 10;
pub const MAX_MAX_ISSUES: usize = 500;
pub const MIN_TIMEOUT_MS: u64 = 100;
pub const MAX_TIMEOUT_MS: u64 = 30_000;
pub const MIN_DEBOUNCE_MS: u64 = 0;
pub const MAX_DEBOUNCE_MS: u64 = 5_000;

/// One user-defined replacement.
///
/// A rule matches a single word, exactly as the user wrote it (apart from case): the
/// checker works word by word, so a rule spanning several words could never fire and is
/// refused instead of being stored as a dead entry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CustomRule {
    /// Word the rule matches, lower case.
    pub pattern: String,
    /// What it is replaced with.
    pub replacement: String,
    /// Whether the rule may be applied by safe auto-correction.
    pub auto_apply: bool,
}

impl CustomRule {
    /// Normalizes and validates a rule.
    ///
    /// Both sides must be a single word: the checker matches one word at a time, so a
    /// phrase could never fire and is refused here instead of being stored as a dead entry.
    pub fn new(
        pattern: &str,
        replacement: &str,
        auto_apply: bool,
    ) -> Result<Self, AutocorrectError> {
        let pattern = normalize_rule_part(pattern)?.to_lowercase();
        let replacement = normalize_rule_part(replacement)?.to_string();
        Ok(Self {
            pattern,
            replacement,
            auto_apply,
        })
    }

    /// Whether the rule looks usable: a pattern and a different replacement.
    pub fn is_usable(&self) -> bool {
        !self.pattern.is_empty()
            && !self.replacement.is_empty()
            && self.pattern.to_lowercase() != self.replacement.to_lowercase()
    }
}

/// Validates one side of a rule: a single word, bounded, without control characters.
fn normalize_rule_part(value: &str) -> Result<&str, AutocorrectError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_DICTIONARY_WORD_CHARS {
        return Err(AutocorrectError::InvalidWord);
    }
    if trimmed
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(AutocorrectError::InvalidWord);
    }
    Ok(trimmed)
}

/// Everything the user can decide about local text correction.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AutocorrectSettings {
    /// Master switch for spelling checks.
    pub enabled: bool,
    /// Check the note editor while typing.
    pub check_notes: bool,
    /// Check the chat draft before it is sent.
    pub check_chat: bool,
    /// Which dictionaries a check uses.
    pub language: LanguageMode,
    /// Also accept a Russian word when the English dictionary knows it, and the
    /// other way round. Off by default, because it hides real mistakes in a
    /// deliberately single-language text.
    pub mixed_mode: bool,
    /// Suggestions offered per issue.
    pub max_suggestions: usize,
    /// Issues one check reports.
    pub max_issues: usize,
    /// How long one check may take.
    pub timeout_ms: u64,
    /// Pause after the last keystroke before a background check runs.
    pub debounce_ms: u64,
    /// Apply unambiguous corrections without asking. Off by default.
    pub safe_autocorrect: bool,
    /// Offer the explicit AI text improvement in notes and chat. Off by default.
    pub ai_improvement: bool,
    /// Show the difference preview before AI changes are applied. Always true in
    /// practice; the field exists so the interface can prove it is on.
    pub require_preview: bool,
    /// Folder that holds the Hunspell pairs. `None` means the application data folder.
    pub dictionary_dir: Option<String>,
    /// Extra word pairs the user defined.
    pub custom_rules: Vec<CustomRule>,
    /// Version of this document, for future migrations.
    pub schema_version: u32,
}

impl Default for AutocorrectSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            check_notes: true,
            check_chat: true,
            language: LanguageMode::Auto,
            mixed_mode: false,
            max_suggestions: DEFAULT_MAX_SUGGESTIONS,
            max_issues: DEFAULT_MAX_ISSUES,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            // Nothing is rewritten automatically until the user asks for it.
            safe_autocorrect: false,
            // The AI part is always an explicit user action.
            ai_improvement: false,
            require_preview: true,
            dictionary_dir: None,
            custom_rules: Vec::new(),
            schema_version: AUTOCORRECT_SETTINGS_SCHEMA_VERSION,
        }
    }
}

impl AutocorrectSettings {
    /// Repairs values that are out of range instead of refusing to load.
    pub fn normalized(mut self) -> Self {
        self.max_suggestions = self
            .max_suggestions
            .clamp(MIN_MAX_SUGGESTIONS, MAX_MAX_SUGGESTIONS);
        self.max_issues = self.max_issues.clamp(MIN_MAX_ISSUES, MAX_MAX_ISSUES);
        self.timeout_ms = self.timeout_ms.clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
        self.debounce_ms = self.debounce_ms.clamp(MIN_DEBOUNCE_MS, MAX_DEBOUNCE_MS);
        // A preview is not optional: it is what makes an AI change a decision.
        self.require_preview = true;
        self.custom_rules.retain(CustomRule::is_usable);
        self.custom_rules.truncate(MAX_CUSTOM_RULES);
        self.dictionary_dir = self.dictionary_dir.and_then(|dir| {
            let trimmed = dir.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        self.schema_version = AUTOCORRECT_SETTINGS_SCHEMA_VERSION;
        self
    }

    /// Whether notes are checked at all.
    pub fn checks_notes(&self) -> bool {
        self.enabled && self.check_notes
    }

    /// Whether the chat draft is checked at all.
    pub fn checks_chat(&self) -> bool {
        self.enabled && self.check_chat
    }

    /// Whether unambiguous corrections may be applied without asking.
    pub fn applies_safely(&self) -> bool {
        self.enabled && self.safe_autocorrect
    }

    /// Whether the AI improvement action is offered.
    pub fn offers_ai_improvement(&self) -> bool {
        self.enabled && self.ai_improvement
    }

    /// Whether a custom rule may be applied without asking.
    pub fn auto_rules(&self) -> impl Iterator<Item = &CustomRule> {
        self.custom_rules
            .iter()
            .filter(|rule| rule.auto_apply && rule.is_usable())
    }

    /// Languages a check consults for one word, in order.
    ///
    /// `mixed_mode` adds the other dictionary as a fallback on top of whichever mode is
    /// selected, so a user who works in two languages can keep the script-based default
    /// and still have the second dictionary consulted.
    pub fn languages_for(&self, word: &str) -> Vec<Language> {
        let mut languages: Vec<Language> = match self.language {
            LanguageMode::Russian => vec![Language::Russian],
            LanguageMode::English => vec![Language::English],
            LanguageMode::Auto => match Language::of_word(word) {
                Some(language) => vec![language],
                None => Vec::new(),
            },
            // The dominating script is tried first, so `ё`/`е` and dash handling stay
            // close to what the user typed, then the other dictionary.
            LanguageMode::Mixed => match Language::of_word(word) {
                Some(Language::Russian) => vec![Language::Russian, Language::English],
                Some(Language::English) => vec![Language::English, Language::Russian],
                None => Vec::new(),
            },
        };
        if self.mixed_mode && languages.len() == 1 {
            let other = match languages[0] {
                Language::Russian => Language::English,
                Language::English => Language::Russian,
            };
            languages.push(other);
        }
        languages
    }

    /// Whether a word may be accepted in either language.
    pub fn checks_both_languages(&self) -> bool {
        self.mixed_mode || self.language.is_mixed()
    }

    pub fn validate(&self) -> Result<(), AutocorrectError> {
        if self.schema_version != AUTOCORRECT_SETTINGS_SCHEMA_VERSION {
            return Err(AutocorrectError::InvalidConfiguration);
        }
        if self != &self.clone().normalized() {
            return Err(AutocorrectError::InvalidConfiguration);
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, AutocorrectError> {
        serde_json::to_string(self).map_err(|_| AutocorrectError::InvalidConfiguration)
    }

    pub fn from_json(text: &str) -> Result<Self, AutocorrectError> {
        let settings: Self =
            serde_json::from_str(text).map_err(|_| AutocorrectError::InvalidConfiguration)?;
        Ok(settings.normalized())
    }
}

/// Reads settings from a stored value, falling back to defaults.
pub fn load_settings(stored: Option<&str>) -> AutocorrectSettings {
    match stored {
        Some(text) if !text.trim().is_empty() => {
            AutocorrectSettings::from_json(text).unwrap_or_default()
        }
        _ => AutocorrectSettings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_check_text_but_never_change_it() {
        let settings = AutocorrectSettings::default();
        assert!(settings.enabled);
        assert!(settings.checks_notes());
        assert!(settings.checks_chat());
        // Nothing is rewritten without the user.
        assert!(!settings.applies_safely());
        // The AI action is explicit.
        assert!(!settings.offers_ai_improvement());
        assert!(settings.require_preview);
        assert!(settings.custom_rules.is_empty());
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn the_master_switch_and_the_ai_switch_gate_every_behaviour() {
        let off = AutocorrectSettings {
            enabled: false,
            safe_autocorrect: true,
            ai_improvement: true,
            ..AutocorrectSettings::default()
        };
        assert!(!off.checks_notes());
        assert!(!off.checks_chat());
        assert!(!off.applies_safely());
        assert!(!off.offers_ai_improvement());
        assert!(off.validate().is_ok());

        let notes_off = AutocorrectSettings {
            check_notes: false,
            ..AutocorrectSettings::default()
        };
        assert!(!notes_off.checks_notes());
        assert!(notes_off.checks_chat());
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let repaired = AutocorrectSettings {
            max_suggestions: 0,
            max_issues: 100_000,
            timeout_ms: 0,
            debounce_ms: 900_000,
            require_preview: false,
            ..AutocorrectSettings::default()
        }
        .normalized();
        assert_eq!(repaired.max_suggestions, MIN_MAX_SUGGESTIONS);
        assert_eq!(repaired.max_issues, MAX_MAX_ISSUES);
        assert_eq!(repaired.timeout_ms, MIN_TIMEOUT_MS);
        assert_eq!(repaired.debounce_ms, MAX_DEBOUNCE_MS);
        // A preview cannot be switched off.
        assert!(repaired.require_preview);
        assert!(repaired.validate().is_ok());

        // The unnormalized value is still refused, so a caller notices.
        let broken = AutocorrectSettings {
            require_preview: false,
            ..AutocorrectSettings::default()
        };
        assert_eq!(
            broken.validate().unwrap_err(),
            AutocorrectError::InvalidConfiguration
        );
    }

    #[test]
    fn a_language_mode_decides_which_dictionaries_a_word_is_tried_against() {
        let auto = AutocorrectSettings::default();
        assert_eq!(auto.languages_for("привет"), vec![Language::Russian]);
        assert_eq!(auto.languages_for("hello"), vec![Language::English]);
        assert!(auto.languages_for("42").is_empty());

        let russian_only = AutocorrectSettings {
            language: LanguageMode::Russian,
            ..AutocorrectSettings::default()
        };
        assert_eq!(russian_only.languages_for("hello"), vec![Language::Russian]);

        let mixed = AutocorrectSettings {
            language: LanguageMode::Mixed,
            ..AutocorrectSettings::default()
        };
        assert_eq!(
            mixed.languages_for("привет"),
            vec![Language::Russian, Language::English]
        );
        assert_eq!(
            mixed.languages_for("hello"),
            vec![Language::English, Language::Russian]
        );
    }

    #[test]
    fn custom_rules_are_validated_and_lowercased() {
        let rule = CustomRule::new("  ДЖАРВИС ", "JARVIS", true).unwrap();
        assert_eq!(rule.pattern, "джарвис");
        assert_eq!(rule.replacement, "JARVIS");
        assert!(rule.auto_apply);
        assert!(rule.is_usable());

        assert!(CustomRule::new("", "x", false).is_err());
        assert!(CustomRule::new("x", "", false).is_err());
        assert!(!CustomRule::new("x", "x", false).unwrap().is_usable());
        assert!(CustomRule::new(&"д".repeat(80), "x", false).is_err());
        // A rule matches one word, so a phrase is refused rather than stored dead.
        assert!(CustomRule::new("в течении", "в течение", false).is_err());
        assert!(CustomRule::new("x", "два слова", false).is_err());
        // A rule that repairs nothing is dropped when settings load.
        let settings = AutocorrectSettings {
            custom_rules: vec![
                CustomRule {
                    pattern: "этта".to_string(),
                    replacement: "это".to_string(),
                    auto_apply: true,
                },
                CustomRule {
                    pattern: "то же".to_string(),
                    replacement: "то же".to_string(),
                    auto_apply: true,
                },
            ],
            ..AutocorrectSettings::default()
        }
        .normalized();
        assert_eq!(settings.custom_rules.len(), 1);
        assert_eq!(settings.auto_rules().count(), 1);
    }

    #[test]
    fn settings_round_trip_and_tolerate_unknown_fields() {
        let settings = AutocorrectSettings {
            language: LanguageMode::Mixed,
            safe_autocorrect: true,
            max_suggestions: 3,
            custom_rules: vec![CustomRule::new("этта", "это", true).unwrap()],
            ..AutocorrectSettings::default()
        };
        let json = settings.to_json().unwrap();
        assert_eq!(AutocorrectSettings::from_json(&json).unwrap(), settings);

        let extended = json.replace("\"schema_version\":1", "\"schema_version\":1,\"future\":7");
        assert!(AutocorrectSettings::from_json(&extended).is_ok());

        assert_eq!(
            AutocorrectSettings::from_json("not json").unwrap_err(),
            AutocorrectError::InvalidConfiguration
        );
        let partial = AutocorrectSettings::from_json(r#"{"debounce_ms":900}"#).unwrap();
        assert_eq!(partial.debounce_ms, 900);
        assert!(partial.checks_notes());
    }

    #[test]
    fn loading_a_damaged_value_yields_defaults() {
        assert_eq!(load_settings(None), AutocorrectSettings::default());
        assert_eq!(load_settings(Some("")), AutocorrectSettings::default());
        assert_eq!(
            load_settings(Some("{ truncated")),
            AutocorrectSettings::default()
        );
        let stored = AutocorrectSettings {
            max_issues: 42,
            ..AutocorrectSettings::default()
        }
        .to_json()
        .unwrap();
        assert_eq!(load_settings(Some(&stored)).max_issues, 42);
    }

    #[test]
    fn more_rules_than_the_limit_are_dropped_not_rejected() {
        let rules: Vec<CustomRule> = (0..MAX_CUSTOM_RULES + 25)
            .map(|index| CustomRule {
                pattern: format!("pattern{index}"),
                replacement: format!("fixed{index}"),
                auto_apply: false,
            })
            .collect();
        let settings = AutocorrectSettings {
            custom_rules: rules,
            ..AutocorrectSettings::default()
        }
        .normalized();
        assert_eq!(settings.custom_rules.len(), MAX_CUSTOM_RULES);
    }
}
