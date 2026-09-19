//! AI-memory settings.
//!
//! The settings themselves are not secret (switches, limits, budgets), so they live
//! in the application settings store next to the local AI configuration. That also
//! means the interface can show and change them while the encrypted storage is
//! locked, which is exactly when a user decides whether to turn memory on at all.
//!
//! Every automatic behaviour that writes memory is **off by default**: the switches
//! that store history and use long-term memory are on, because the user asked for
//! the feature, but suggesting new facts from a conversation is not. Nothing a
//! model produces becomes memory without a user action.

use serde::{Deserialize, Serialize};

use super::error::MemoryError;
use super::model::MemoryStats;

/// Settings key used by the existing application settings store (`app.db`).
pub const SETTINGS_KEY: &str = "ai_memory_settings";
/// Schema version of the settings document.
pub const MEMORY_SETTINGS_SCHEMA_VERSION: u32 = 1;

/// Default number of recent messages handed to the model verbatim.
pub const DEFAULT_MAX_RECENT_MESSAGES: usize = 12;
/// Default token budget for long-term facts.
pub const DEFAULT_MEMORY_TOKEN_BUDGET: usize = 512;
/// Default number of uncovered messages that triggers a summary.
pub const DEFAULT_SUMMARY_TRIGGER_MESSAGES: usize = 12;
/// Default number of newest messages always kept verbatim.
pub const DEFAULT_SUMMARY_KEEP_RECENT: usize = 6;

/// Bounds enforced by [`MemorySettings::normalized`].
pub const MIN_MAX_RECENT_MESSAGES: usize = 2;
pub const MAX_MAX_RECENT_MESSAGES: usize = 100;
pub const MIN_MEMORY_TOKEN_BUDGET: usize = 0;
pub const MAX_MEMORY_TOKEN_BUDGET: usize = 4096;
pub const MIN_SUMMARY_TRIGGER_MESSAGES: usize = 4;
pub const MAX_SUMMARY_TRIGGER_MESSAGES: usize = 200;
pub const MIN_SUMMARY_KEEP_RECENT: usize = 2;
pub const MAX_SUMMARY_KEEP_RECENT: usize = 50;

/// Above this many facts the interface warns about the linear search cost.
pub const LINEAR_SEARCH_WARNING_FACTS: usize = 500;
/// Above this many stored messages the interface warns as well.
pub const LINEAR_SEARCH_WARNING_MESSAGES: usize = 5000;

/// Everything the user can decide about AI memory.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct MemorySettings {
    /// Master switch. When off, nothing is stored and nothing is used.
    pub enabled: bool,
    /// Store conversations and messages at all.
    pub save_history: bool,
    /// Use approved long-term facts when building a request.
    pub use_long_term_memory: bool,
    /// Let the model propose candidates after an answer. Off by default.
    pub suggest_facts: bool,
    /// Write summaries of older messages once the threshold is reached.
    pub auto_summaries: bool,
    /// Newest messages always sent verbatim.
    pub max_recent_messages: usize,
    /// Cap for the token budget of the fact section.
    pub memory_token_budget: usize,
    /// Uncovered messages that trigger a summary.
    pub summary_trigger_messages: usize,
    /// Newest messages a summary never covers.
    pub summary_keep_recent: usize,
    /// Version of this document, for future migrations.
    pub schema_version: u32,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            save_history: true,
            use_long_term_memory: true,
            // Automatic extraction is opt-in: a model must not decide what to
            // remember on its own.
            suggest_facts: false,
            auto_summaries: true,
            max_recent_messages: DEFAULT_MAX_RECENT_MESSAGES,
            memory_token_budget: DEFAULT_MEMORY_TOKEN_BUDGET,
            summary_trigger_messages: DEFAULT_SUMMARY_TRIGGER_MESSAGES,
            summary_keep_recent: DEFAULT_SUMMARY_KEEP_RECENT,
            schema_version: MEMORY_SETTINGS_SCHEMA_VERSION,
        }
    }
}

impl MemorySettings {
    /// Repairs values that are out of range instead of refusing to load.
    ///
    /// A settings file written by another build, or edited by hand, must never stop
    /// the application; it is clamped into the documented ranges and stored back on
    /// the next save.
    pub fn normalized(self) -> Self {
        Self {
            enabled: self.enabled,
            save_history: self.save_history,
            use_long_term_memory: self.use_long_term_memory,
            suggest_facts: self.suggest_facts,
            auto_summaries: self.auto_summaries,
            max_recent_messages: clamp(
                self.max_recent_messages,
                MIN_MAX_RECENT_MESSAGES,
                MAX_MAX_RECENT_MESSAGES,
                DEFAULT_MAX_RECENT_MESSAGES,
            ),
            memory_token_budget: clamp(
                self.memory_token_budget,
                MIN_MEMORY_TOKEN_BUDGET,
                MAX_MEMORY_TOKEN_BUDGET,
                DEFAULT_MEMORY_TOKEN_BUDGET,
            ),
            summary_trigger_messages: clamp(
                self.summary_trigger_messages,
                MIN_SUMMARY_TRIGGER_MESSAGES,
                MAX_SUMMARY_TRIGGER_MESSAGES,
                DEFAULT_SUMMARY_TRIGGER_MESSAGES,
            ),
            summary_keep_recent: clamp(
                self.summary_keep_recent,
                MIN_SUMMARY_KEEP_RECENT,
                MAX_SUMMARY_KEEP_RECENT,
                DEFAULT_SUMMARY_KEEP_RECENT,
            ),
            schema_version: MEMORY_SETTINGS_SCHEMA_VERSION,
        }
    }

    /// Whether history may be written for this conversation.
    pub fn stores_history(&self) -> bool {
        self.enabled && self.save_history
    }

    /// Whether approved facts may be used in a request.
    pub fn uses_long_term_memory(&self) -> bool {
        self.enabled && self.use_long_term_memory
    }

    /// Whether the model may suggest candidates after an answer.
    pub fn suggests_facts(&self) -> bool {
        self.enabled && self.suggest_facts
    }

    /// Whether summaries may be written.
    pub fn writes_summaries(&self) -> bool {
        self.enabled && self.save_history && self.auto_summaries
    }

    /// Whether enough uncovered messages have accumulated for a summary.
    ///
    /// `uncovered` must be larger than the number of messages that stay verbatim,
    /// so a summary never covers the newest part of the conversation.
    pub fn should_summarize(&self, uncovered: usize) -> bool {
        self.writes_summaries()
            && uncovered >= self.summary_trigger_messages
            && uncovered > self.summary_keep_recent
    }

    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema_version != MEMORY_SETTINGS_SCHEMA_VERSION {
            return Err(MemoryError::InvalidConfiguration);
        }
        if self != &self.normalized() {
            return Err(MemoryError::InvalidConfiguration);
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, MemoryError> {
        serde_json::to_string(self).map_err(|_| MemoryError::InvalidConfiguration)
    }

    pub fn from_json(text: &str) -> Result<Self, MemoryError> {
        let settings: Self =
            serde_json::from_str(text).map_err(|_| MemoryError::InvalidConfiguration)?;
        Ok(settings.normalized())
    }
}

/// Reads settings from a stored value, falling back to defaults.
///
/// A missing, empty, or damaged value means defaults: a bad settings string must
/// never stop the application from starting.
pub fn load_settings(stored: Option<&str>) -> MemorySettings {
    match stored {
        Some(text) if !text.trim().is_empty() => {
            MemorySettings::from_json(text).unwrap_or_default()
        }
        _ => MemorySettings::default(),
    }
}

fn clamp(value: usize, min: usize, max: usize, fallback: usize) -> usize {
    if value == 0 && fallback != 0 && min > 0 {
        // Zero is only meaningful where the range allows it.
        return min;
    }
    value.clamp(min, max)
}

/// A warning about the linear cost of search, when memory grew large.
///
/// The store decrypts and scans every entry in memory, which is fast for a personal
/// assistant and slow for a corpus. The interface shows this before it becomes a
/// surprise; a vector index is deliberately not part of this stage.
pub fn linear_search_warning(stats: &MemoryStats) -> Option<&'static str> {
    if stats.facts > LINEAR_SEARCH_WARNING_FACTS {
        return Some("memory_search_cost_facts");
    }
    if stats.messages > LINEAR_SEARCH_WARNING_MESSAGES {
        return Some("memory_search_cost_messages");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_conservative_and_never_extract_automatically() {
        let settings = MemorySettings::default();
        assert!(settings.enabled);
        assert!(settings.save_history);
        assert!(settings.use_long_term_memory);
        // The one behaviour that writes memory from model output is off.
        assert!(!settings.suggest_facts);
        assert!(!settings.suggests_facts());
        assert!(settings.auto_summaries);
        assert_eq!(settings.max_recent_messages, DEFAULT_MAX_RECENT_MESSAGES);
        assert_eq!(settings.memory_token_budget, DEFAULT_MEMORY_TOKEN_BUDGET);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn the_master_switch_disables_every_other_behaviour() {
        let off = MemorySettings {
            enabled: false,
            ..MemorySettings::default()
        };
        assert!(!off.stores_history());
        assert!(!off.uses_long_term_memory());
        assert!(!off.suggests_facts());
        assert!(!off.writes_summaries());
        assert!(!off.should_summarize(1000));
        assert!(off.validate().is_ok());
    }

    #[test]
    fn out_of_range_values_are_clamped_instead_of_refused() {
        let repaired = MemorySettings {
            max_recent_messages: 0,
            memory_token_budget: 100_000,
            summary_trigger_messages: 1,
            summary_keep_recent: 10_000,
            ..MemorySettings::default()
        }
        .normalized();
        assert_eq!(repaired.max_recent_messages, MIN_MAX_RECENT_MESSAGES);
        assert_eq!(repaired.memory_token_budget, MAX_MEMORY_TOKEN_BUDGET);
        assert_eq!(
            repaired.summary_trigger_messages,
            MIN_SUMMARY_TRIGGER_MESSAGES
        );
        assert_eq!(repaired.summary_keep_recent, MAX_SUMMARY_KEEP_RECENT);
        assert!(repaired.validate().is_ok());

        // The original value is still refused, so a caller notices.
        let broken = MemorySettings {
            max_recent_messages: 0,
            ..MemorySettings::default()
        };
        assert_eq!(
            broken.validate().unwrap_err(),
            MemoryError::InvalidConfiguration
        );
    }

    #[test]
    fn settings_round_trip_and_tolerate_unknown_fields() {
        let settings = MemorySettings {
            suggest_facts: true,
            memory_token_budget: 1024,
            ..MemorySettings::default()
        };
        let json = settings.to_json().unwrap();
        assert_eq!(MemorySettings::from_json(&json).unwrap(), settings);

        let extended = json.replace("\"schema_version\":1", "\"schema_version\":1,\"future\":7");
        assert!(MemorySettings::from_json(&extended).is_ok());

        assert_eq!(
            MemorySettings::from_json("not json").unwrap_err(),
            MemoryError::InvalidConfiguration
        );
        let partial = MemorySettings::from_json(r#"{"memory_token_budget":2048}"#).unwrap();
        assert_eq!(partial.memory_token_budget, 2048);
        assert_eq!(partial.max_recent_messages, DEFAULT_MAX_RECENT_MESSAGES);
    }

    #[test]
    fn loading_a_damaged_value_yields_defaults() {
        assert_eq!(load_settings(None), MemorySettings::default());
        assert_eq!(load_settings(Some("")), MemorySettings::default());
        assert_eq!(
            load_settings(Some("{ truncated")),
            MemorySettings::default()
        );
        let stored = MemorySettings {
            max_recent_messages: 20,
            ..MemorySettings::default()
        }
        .to_json()
        .unwrap();
        assert_eq!(load_settings(Some(&stored)).max_recent_messages, 20);
    }

    #[test]
    fn a_summary_needs_enough_uncovered_messages_and_keeps_the_newest_ones() {
        let settings = MemorySettings::default();
        assert!(!settings.should_summarize(settings.summary_trigger_messages - 1));
        assert!(settings.should_summarize(settings.summary_trigger_messages));
        // Even with many messages, the newest ones stay verbatim.
        let tight = MemorySettings {
            summary_trigger_messages: 4,
            summary_keep_recent: 8,
            ..MemorySettings::default()
        };
        assert!(!tight.should_summarize(6));
        assert!(tight.should_summarize(9));

        let no_summaries = MemorySettings {
            auto_summaries: false,
            ..MemorySettings::default()
        };
        assert!(!no_summaries.should_summarize(1000));
    }

    #[test]
    fn large_memory_produces_a_linear_cost_warning() {
        let small = MemoryStats::default();
        assert!(linear_search_warning(&small).is_none());

        let many_facts = MemoryStats {
            facts: LINEAR_SEARCH_WARNING_FACTS + 1,
            ..MemoryStats::default()
        };
        assert_eq!(
            linear_search_warning(&many_facts),
            Some("memory_search_cost_facts")
        );

        let many_messages = MemoryStats {
            messages: LINEAR_SEARCH_WARNING_MESSAGES + 1,
            ..MemoryStats::default()
        };
        assert_eq!(
            linear_search_warning(&many_messages),
            Some("memory_search_cost_messages")
        );
    }
}
