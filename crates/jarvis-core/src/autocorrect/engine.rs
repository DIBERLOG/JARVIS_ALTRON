//! The checker: tokenize, look each word up, and report what does not match.
//!
//! The order of decisions matters, and it is what keeps the report trustworthy:
//!
//! 1. **Skip** everything that is not clearly a word (code, URLs, numbers,
//!    abbreviations) — the tokenizer does that before this module sees anything.
//! 2. **Accept** a word the user's own encrypted dictionary knows.
//! 3. **Accept** a word the installed Hunspell dictionary knows, including the other
//!    `ё`/`е` spelling of it, because both spellings are correct Russian.
//! 4. Only then report an issue, with suggestions. A word nobody recognises is reported
//!    as a question with nothing invented around it: the checker never proposes a
//!    replacement it cannot justify from a dictionary, the user's own words, or a rule
//!    the user wrote.
//!
//! The checker holds no state of its own beyond the dictionary cache, so one instance can
//! be shared while the encrypted word list stays where the lock can drop it: every call
//! borrows the [`UserDictionaryStore`] of the unlocked session.
//!
//! Everything here is bounded on purpose: a text larger than [`MAX_CHECK_CHARS`] is
//! refused, a check that runs past the configured timeout stops early and says so, and
//! the number of issues is capped. Nothing blocks, and no lock is held while a model
//! generates text.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::sync::{CryptoProvider, SyncRepository};

use super::dictionary::{DictionaryManager, LoadedDictionary};
use super::error::AutocorrectError;
use super::model::{
    is_double_capital, text_version, yo_variant, Correction, IssueReason, Language, SpellingIssue,
    Suggestion, SuggestionSource, TextRange, MAX_CHECK_CHARS, MAX_ISSUES, MAX_SUGGESTIONS,
    MAX_TOKEN_CHARS,
};
use super::settings::AutocorrectSettings;
use super::tokenizer::{scan_with_limit, ScanResult, WordToken};
use super::user_dictionary::UserDictionaryStore;

/// What one check found.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CheckReport {
    /// Version of the checked text, from [`text_version`].
    pub version: String,
    /// Whether checking was switched on at all.
    pub enabled: bool,
    pub issues: Vec<SpellingIssue>,
    /// Issues that safe auto-correction may fix without asking.
    pub auto_fixable: usize,
    pub words_checked: usize,
    /// Words the tokenizer left alone, counted so the interface can explain itself.
    pub words_skipped: usize,
    /// Words whose language has no dictionary installed: nothing was claimed about them.
    pub words_unverified: usize,
    /// Languages whose dictionary is missing, so the interface can offer to install one.
    pub unavailable: Vec<Language>,
    /// Whether the user's own word list was available. False while the storage is
    /// locked: the check then runs against the dictionaries alone, and the interface
    /// says so instead of presenting an incomplete result as complete.
    pub user_dictionary_available: bool,
    /// Whether the issue list was cut short by the configured limit.
    pub truncated: bool,
    /// Whether the check stopped because its time budget ran out.
    pub timed_out: bool,
    pub elapsed_ms: u64,
}

impl CheckReport {
    /// Whether nothing was reported.
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }

    /// Whether at least one dictionary was available for this check.
    pub fn has_dictionary(&self) -> bool {
        self.unavailable.len() < Language::all().len()
    }

    /// The reported words, in text order.
    pub fn issue_words(&self) -> Vec<&str> {
        self.issues
            .iter()
            .map(|issue| issue.word.as_str())
            .collect()
    }
}

/// Whether a word could be judged.
enum Knownness {
    Known,
    Unknown,
    /// No dictionary is installed for this word's language: say nothing about it.
    Unjudgeable,
}

/// The local checker: dictionaries on disk, and the user's word list passed per call.
///
/// The dictionary manager is shared behind an [`Arc`], because its own caches are
/// lock-protected: a checker can be built cheaply for one command while the folder and
/// the parsed dictionaries stay loaded across commands.
pub struct LocalSpellChecker {
    dictionaries: Arc<DictionaryManager>,
    /// Text size above which a check is refused.
    max_chars: usize,
}

impl LocalSpellChecker {
    pub fn new(dictionaries: Arc<DictionaryManager>) -> Self {
        Self {
            dictionaries,
            max_chars: MAX_CHECK_CHARS,
        }
    }

    /// Builds a checker that owns its dictionary manager.
    pub fn from_manager(dictionaries: DictionaryManager) -> Self {
        Self::new(Arc::new(dictionaries))
    }

    /// Overrides the accepted text size, for tests.
    pub fn with_max_chars(mut self, max_chars: usize) -> Self {
        self.max_chars = max_chars;
        self
    }

    /// The shared dictionary manager.
    pub fn dictionaries(&self) -> &Arc<DictionaryManager> {
        &self.dictionaries
    }

    /// The folder the dictionaries are read from.
    pub fn directory(&self) -> &std::path::Path {
        self.dictionaries.directory()
    }
    /// Checks a text against the dictionaries and the user's own word list.
    ///
    /// `user` is optional on purpose: while the encrypted storage is locked its words
    /// cannot be read, and the check still works against the dictionaries alone. The
    /// caller is expected to tell the user that their own words were left out.
    pub fn check<R: SyncRepository, C: CryptoProvider>(
        &self,
        user: Option<&mut UserDictionaryStore<R, C>>,
        text: &str,
        settings: &AutocorrectSettings,
    ) -> Result<CheckReport, AutocorrectError> {
        let mut report = CheckReport {
            version: text_version(text),
            enabled: settings.enabled,
            ..CheckReport::default()
        };
        if !settings.enabled {
            // Switching the checker off is a decision, not an error.
            return Ok(report);
        }
        if text.chars().count() > self.max_chars {
            return Err(AutocorrectError::TextTooLarge {
                limit: self.max_chars,
            });
        }

        report.user_dictionary_available = user.is_some();
        // One mutable load, then a read-only view for the whole text: the list is
        // decrypted once and the walk below never needs a mutable borrow.
        let mut user = user;
        if let Some(store) = user.as_mut() {
            store.prepare()?;
        }
        let user: Option<&UserDictionaryStore<R, C>> = user.as_deref();
        let started = Instant::now();
        let deadline = started + Duration::from_millis(settings.timeout_ms);
        let scan = scan_with_limit(text, MAX_TOKEN_CHARS);
        report.words_skipped = scan.skipped.len();
        report.unavailable = Language::all()
            .iter()
            .copied()
            .filter(|language| self.dictionary(*language).is_none())
            .collect();

        let limit = settings.max_issues.min(MAX_ISSUES);
        let mut issues = punctuation_issues(text);
        issues.extend(self.word_issues(user, text, &scan, settings, deadline, &mut report));
        // Text order, so the interface can walk the report from the start.
        issues.sort_by_key(|issue| (issue.range.start, issue.range.end));
        for issue in issues {
            if report.issues.len() >= limit {
                report.truncated = true;
                break;
            }
            if issue.auto_fixable {
                report.auto_fixable += 1;
            }
            report.issues.push(issue);
        }
        report.elapsed_ms = started.elapsed().as_millis() as u64;
        Ok(report)
    }

    /// The corrections safe auto-correction may apply without asking.
    ///
    /// Only issues the settings mark as unambiguous become corrections, and an unknown
    /// word is never among them whatever the settings say: a name can be a valid word
    /// the dictionary does not know.
    pub fn safe_corrections(
        report: &CheckReport,
        settings: &AutocorrectSettings,
    ) -> Vec<Correction> {
        if !settings.applies_safely() {
            return Vec::new();
        }
        report
            .issues
            .iter()
            .filter(|issue| issue.auto_fixable && issue.reason.is_auto_fixable())
            .filter_map(|issue| {
                let replacement = issue.best_suggestion()?.to_string();
                (replacement != issue.word).then(|| Correction {
                    range: issue.range,
                    original: issue.word.clone(),
                    replacement,
                    reason: Some(issue.reason),
                })
            })
            .collect()
    }

    /// Suggestions for one word, as the interface asks for a single correction.
    ///
    /// `user` is optional for the same reason as in [`LocalSpellChecker::check`].
    pub fn suggest<R: SyncRepository, C: CryptoProvider>(
        &self,
        mut user: Option<&mut UserDictionaryStore<R, C>>,
        word: &str,
        settings: &AutocorrectSettings,
    ) -> Result<Vec<Suggestion>, AutocorrectError> {
        if let Some(store) = user.as_mut() {
            store.prepare()?;
        }
        let view: Option<&UserDictionaryStore<R, C>> = user.as_deref();
        Ok(self.suggestions_for(view, word, settings))
    }

    // ---------------------------------------------------------------- internals

    /// One dictionary, loaded on first use.
    fn dictionary(&self, language: Language) -> Option<Arc<LoadedDictionary>> {
        self.dictionaries.load(language).ok()
    }

    fn word_issues<R: SyncRepository, C: CryptoProvider>(
        &self,
        user: Option<&UserDictionaryStore<R, C>>,
        text: &str,
        scan: &ScanResult,
        settings: &AutocorrectSettings,
        deadline: Instant,
        report: &mut CheckReport,
    ) -> Vec<SpellingIssue> {
        let mut issues = Vec::new();
        for token in &scan.words {
            if Instant::now() >= deadline {
                report.timed_out = true;
                break;
            }
            if user.is_some_and(|words| words.is_ignored(&token.text)) {
                report.words_skipped += 1;
                continue;
            }
            report.words_checked += 1;
            match self.classify(user, &token.text, settings) {
                Knownness::Known => continue,
                Knownness::Unjudgeable => {
                    report.words_unverified += 1;
                    continue;
                }
                Knownness::Unknown => {}
            }
            let suggestions = self.suggestions_for(user, &token.text, settings);
            let (reason, auto_fixable) = reason_for(&token.text, settings);
            if let Some(issue) = build_issue(text, token, suggestions, reason, auto_fixable) {
                issues.push(issue);
            }
        }
        issues
    }

    /// Whether the checker knows this word, and whether it is able to judge it at all.
    fn classify<R: SyncRepository, C: CryptoProvider>(
        &self,
        user: Option<&UserDictionaryStore<R, C>>,
        word: &str,
        settings: &AutocorrectSettings,
    ) -> Knownness {
        if user.is_some_and(|words| words.known(word)) {
            return Knownness::Known;
        }
        let languages = settings.languages_for(word);
        if languages.is_empty() {
            // A token with no letters in either script is not a word for this checker.
            return Knownness::Known;
        }
        let mut any_dictionary = false;
        for language in languages {
            let Some(dictionary) = self.dictionary(language) else {
                continue;
            };
            any_dictionary = true;
            if dictionary.check(word) {
                return Knownness::Known;
            }
            // Both `ё` and `е` spellings are correct Russian, so a word is accepted
            // when the dictionary knows the other spelling of it. The English
            // dictionary is only asked about `ё`/`е` in mixed mode, where it is one of
            // the two languages the user works in.
            if language == Language::Russian || settings.checks_both_languages() {
                if let Some(variant) = yo_variant(word) {
                    if dictionary.check(&variant) {
                        return Knownness::Known;
                    }
                }
            }
        }
        if any_dictionary {
            Knownness::Unknown
        } else {
            Knownness::Unjudgeable
        }
    }

    /// Suggestions for one word, ordered by how likely they are to be what was meant.
    fn suggestions_for<R: SyncRepository, C: CryptoProvider>(
        &self,
        user: Option<&UserDictionaryStore<R, C>>,
        word: &str,
        settings: &AutocorrectSettings,
    ) -> Vec<Suggestion> {
        let limit = settings.max_suggestions.min(MAX_SUGGESTIONS);
        let mut suggestions: Vec<Suggestion> = Vec::new();
        let normalized = word.trim().to_lowercase();

        // 1. A user rule is the user's own instruction, so it comes first.
        for rule in &settings.custom_rules {
            if rule.pattern != normalized {
                continue;
            }
            push_suggestion(
                &mut suggestions,
                adapt_capitalisation(word, &rule.replacement),
                SuggestionSource::UserRule,
            );
        }

        // 2. The user's own words, which are the most personal signal available. While
        //    the storage is locked there is no list, and the dictionaries carry the check.
        if let Some(words) = user {
            for language in settings.languages_for(word) {
                let candidates = words.loaded_suggestions(word, language, limit);
                for candidate in candidates {
                    let source = match yo_variant(word) {
                        Some(variant) if variant.eq_ignore_ascii_case(&candidate) => {
                            SuggestionSource::YoVariant
                        }
                        _ => SuggestionSource::UserDictionary,
                    };
                    push_suggestion(
                        &mut suggestions,
                        adapt_capitalisation(word, &candidate),
                        source,
                    );
                }
            }
        }

        // 3. The dictionaries.
        for language in settings.languages_for(word) {
            let Some(dictionary) = self.dictionary(language) else {
                continue;
            };
            for candidate in dictionary.suggestions(word, limit) {
                push_suggestion(
                    &mut suggestions,
                    adapt_capitalisation(word, &candidate),
                    SuggestionSource::Dictionary,
                );
            }
        }

        // 4. The other `ё`/`е` spelling of the same word, when a dictionary lists it
        //    even though this exact spelling was not found.
        if let Some(variant) = yo_variant(word) {
            let known = settings.languages_for(word).iter().any(|language| {
                self.dictionary(*language)
                    .is_some_and(|dictionary| dictionary.check(&variant))
            });
            if known {
                push_suggestion(
                    &mut suggestions,
                    adapt_capitalisation(word, &variant),
                    SuggestionSource::YoVariant,
                );
            }
        }

        suggestions.truncate(limit);
        suggestions
    }
}

impl std::fmt::Debug for LocalSpellChecker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalSpellChecker")
            .field("dictionaries", &self.dictionaries)
            .field("max_chars", &self.max_chars)
            .finish()
    }
}

/// Adds a suggestion unless it is empty or already listed.
fn push_suggestion(suggestions: &mut Vec<Suggestion>, text: String, source: SuggestionSource) {
    if text.is_empty() {
        return;
    }
    if suggestions
        .iter()
        .any(|existing| existing.text.to_lowercase() == text.to_lowercase())
    {
        return;
    }
    suggestions.push(Suggestion { text, source });
}

/// The only repair of a double capital, which has exactly one sensible answer.
///
/// The rule for *recognising* one lives in the model ([`is_double_capital`]), because the
/// tokenizer needs it as well; this is the rewrite it implies.
pub fn fix_double_capital(word: &str) -> Option<String> {
    if !is_double_capital(word) {
        return None;
    }
    let mut characters = word.chars();
    let first = characters.next()?;
    let rest = characters.as_str().to_lowercase();
    let mut fixed = String::with_capacity(word.len());
    fixed.push(first);
    fixed.push_str(&rest);
    Some(fixed)
}

/// Why a word was reported, and whether it may be repaired without asking.
fn reason_for(word: &str, settings: &AutocorrectSettings) -> (IssueReason, bool) {
    let normalized = word.trim().to_lowercase();
    for rule in &settings.custom_rules {
        if rule.pattern == normalized {
            return (IssueReason::UserRule, rule.auto_apply);
        }
    }
    if is_double_capital(word) {
        return (IssueReason::DoubleCapital, true);
    }
    // Anything else is a word no dictionary knows; the user decides what to do.
    (IssueReason::UnknownWord, false)
}

/// Keeps the capitalisation the user used when the suggestion is all lower case.
pub fn adapt_capitalisation(original: &str, replacement: &str) -> String {
    let Some(first) = original.chars().next() else {
        return replacement.to_string();
    };
    if !first.is_uppercase() {
        return replacement.to_string();
    }
    let mut replacement_chars = replacement.chars();
    let Some(replacement_first) = replacement_chars.next() else {
        return replacement.to_string();
    };
    if replacement_first.is_uppercase() {
        return replacement.to_string();
    }
    let mut adapted = String::with_capacity(replacement.len() + 1);
    adapted.extend(replacement_first.to_uppercase());
    adapted.push_str(replacement_chars.as_str());
    adapted
}

/// Punctuation issues that need no dictionary: repeated spaces and a space before a
/// punctuation mark. Both have exactly one sensible repair, so both may be applied by
/// safe auto-correction when the user switched it on.
pub fn punctuation_issues(text: &str) -> Vec<SpellingIssue> {
    let mut issues = Vec::new();
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < characters.len() {
        if characters[index] == ' ' {
            let start = index;
            while index < characters.len() && characters[index] == ' ' {
                index += 1;
            }
            if index - start >= 2 {
                push_range_issue(
                    &mut issues,
                    text,
                    TextRange::new(start, index),
                    IssueReason::RepeatedSpace,
                    " ".to_string(),
                );
            }
            continue;
        }
        index += 1;
    }
    // A space directly before `,.;:!?` is never wanted, and the repair is a deletion.
    for (index, character) in characters.iter().enumerate() {
        if matches!(character, ',' | '.' | ';' | ':' | '!' | '?' | '%')
            && index > 0
            && characters[index - 1] == ' '
        {
            push_range_issue(
                &mut issues,
                text,
                TextRange::new(index - 1, index),
                IssueReason::SpaceBeforePunctuation,
                String::new(),
            );
        }
    }
    // Text order, so a caller can walk the list from the start.
    issues.sort_by_key(|issue| issue.range.start);
    issues
}

fn push_range_issue(
    issues: &mut Vec<SpellingIssue>,
    text: &str,
    range: TextRange,
    reason: IssueReason,
    replacement: String,
) {
    let Ok(original) = range.slice(text) else {
        return;
    };
    let Ok(utf16) = range.to_utf16(text) else {
        return;
    };
    issues.push(SpellingIssue {
        word: original.to_string(),
        range,
        utf16,
        // A punctuation rule belongs to no language.
        language: None,
        // An empty replacement means "delete this text", which `Correction` expresses
        // directly and which the interface shows as a deletion in the preview.
        suggestions: vec![Suggestion {
            text: replacement,
            source: SuggestionSource::TextRule,
        }],
        reason,
        auto_fixable: true,
    });
}

/// Builds the reported issue, adding the double-capital repair when it applies.
fn build_issue(
    text: &str,
    token: &WordToken,
    mut suggestions: Vec<Suggestion>,
    reason: IssueReason,
    auto_fixable: bool,
) -> Option<SpellingIssue> {
    if reason == IssueReason::DoubleCapital {
        if let Some(fixed) = fix_double_capital(&token.text) {
            // The repair is the first suggestion, so "apply the suggestion" and
            // "apply the only possible repair" are the same action.
            suggestions.insert(
                0,
                Suggestion {
                    text: fixed,
                    source: SuggestionSource::TextRule,
                },
            );
        }
    }
    let utf16 = token.range.to_utf16(text).ok()?;
    Some(SpellingIssue {
        word: token.text.clone(),
        range: token.range,
        utf16,
        language: token.language,
        suggestions,
        reason,
        // A double capital is always repairable; a user rule only when the user said so.
        auto_fixable: auto_fixable || reason == IssueReason::DoubleCapital,
    })
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use crate::sync::crypto::{random_master_key, MasterKeyCryptoProvider};
    use crate::sync::{DeviceId, InMemorySyncRepository};

    /// The word list used by the unit tests: real crypto, no disk, no network.
    pub(crate) type TestUserDictionary =
        UserDictionaryStore<InMemorySyncRepository, MasterKeyCryptoProvider>;

    /// A user dictionary with a fresh key and an in-memory repository.
    pub(crate) fn fixture_user() -> TestUserDictionary {
        UserDictionaryStore::new(
            InMemorySyncRepository::new(),
            MasterKeyCryptoProvider::new(random_master_key().unwrap()),
            DeviceId::new("autocorrect_test_device").unwrap(),
        )
    }

    /// Writes the tiny dictionary pair the unit tests use.
    ///
    /// It is a fixture written by the test itself, not a shipped dictionary: the real
    /// Russian and English lists are installed by the user and never committed.
    pub(crate) fn write_fixture_dictionary(directory: &std::path::Path) {
        std::fs::create_dir_all(directory).unwrap();
        let aff = "SET UTF-8\n";
        let dic = "16\nпривет\nмир\nдела\nкак\nчто\nэто\nи\nоткрой\nштуки\nприветствие\nёж\n\
                   hello\nworld\ngood\nnotes\nid\n";
        std::fs::write(directory.join("ru_RU.aff"), aff).unwrap();
        std::fs::write(directory.join("ru_RU.dic"), dic).unwrap();
        std::fs::write(directory.join("en_US.aff"), aff).unwrap();
        std::fs::write(directory.join("en_US.dic"), dic).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::*;
    use super::*;

    /// A checker over a folder with the fixture dictionary, and a fresh word list.
    fn fixture() -> (tempfile::TempDir, LocalSpellChecker, TestUserDictionary) {
        let directory = tempfile::tempdir().unwrap();
        write_fixture_dictionary(directory.path());
        let checker = LocalSpellChecker::from_manager(DictionaryManager::new(directory.path()));
        (directory, checker, fixture_user())
    }

    #[test]
    fn a_double_capital_word_has_one_repair() {
        assert_eq!(fix_double_capital("HEllo").as_deref(), Some("Hello"));
        assert_eq!(fix_double_capital("ПРивет").as_deref(), Some("Привет"));
        assert_eq!(fix_double_capital("Hello"), None);
        assert_eq!(fix_double_capital("API"), None);
        assert_eq!(fix_double_capital("McDonald"), None);
        assert!(is_double_capital("HEl"));
        assert!(!is_double_capital("HE"));
        // An identifier with two leading capitals is not a typo.
        assert!(!is_double_capital("HTTPServer"));
    }

    #[test]
    fn capitalisation_of_a_suggestion_follows_the_word() {
        assert_eq!(adapt_capitalisation("Привт", "привет"), "Привет");
        assert_eq!(adapt_capitalisation("привт", "привет"), "привет");
        // The user's capital is kept, so a corrected word does not change the sentence.
        assert_eq!(adapt_capitalisation("Helo", "hello"), "Hello");
        assert_eq!(adapt_capitalisation("Helo", ""), "");
        assert_eq!(adapt_capitalisation("", "hello"), "hello");
    }

    #[test]
    fn suggestions_are_unique_and_case_insensitive() {
        let mut suggestions = Vec::new();
        push_suggestion(
            &mut suggestions,
            "привет".to_string(),
            SuggestionSource::Dictionary,
        );
        push_suggestion(
            &mut suggestions,
            "Привет".to_string(),
            SuggestionSource::UserDictionary,
        );
        push_suggestion(
            &mut suggestions,
            String::new(),
            SuggestionSource::Dictionary,
        );
        assert_eq!(suggestions.len(), 1);
    }

    #[test]
    fn punctuation_is_repaired_only_where_the_answer_is_obvious() {
        let issues = punctuation_issues("Привет ,  мир");
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0].reason, IssueReason::SpaceBeforePunctuation);
        assert!(issues[0].auto_fixable);
        assert_eq!(issues[0].suggestions[0].text, "");
        assert_eq!(issues[0].range.slice("Привет ,  мир").unwrap(), " ");
        assert_eq!(issues[1].reason, IssueReason::RepeatedSpace);
        assert_eq!(issues[1].suggestions[0].text, " ");
        assert_eq!(issues[1].range.slice("Привет ,  мир").unwrap(), "  ");
        assert_eq!(issues[1].suggestions[0].source, SuggestionSource::TextRule);
        assert!(punctuation_issues("Обычный текст.").is_empty());
        assert_eq!(punctuation_issues("много     пробелов").len(), 1);
        // A space after punctuation is correct and is not reported.
        assert!(punctuation_issues("Привет, мир!").is_empty());
    }

    #[test]
    fn a_correct_text_produces_no_issues() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        let text = "Привет, мир! Как дела? Hello, world!";
        let report = checker.check(Some(&mut user), text, &settings).unwrap();
        assert!(report.is_clean(), "{:?}", report.issue_words());
        assert!(report.enabled);
        assert!(report.words_checked >= 6);
        assert!(report.has_dictionary());
        assert!(!report.timed_out);
        assert!(!report.truncated);
        assert_eq!(report.version, text_version(text));
        assert_eq!(report.auto_fixable, 0);
    }

    #[test]
    fn a_misspelled_word_is_reported_with_suggestions_from_the_dictionary() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        let text = "привт мир";
        let report = checker.check(Some(&mut user), text, &settings).unwrap();
        assert_eq!(report.issue_words(), vec!["привт"]);
        let issue = &report.issues[0];
        assert_eq!(issue.reason, IssueReason::UnknownWord);
        assert!(!issue.auto_fixable);
        assert_eq!(issue.range.slice(text).unwrap(), "привт");
        assert_eq!(issue.utf16.start, 0);
        assert!(issue
            .suggestions
            .iter()
            .any(|suggestion| suggestion.text == "привет"));
        assert_eq!(report.auto_fixable, 0);
    }

    #[test]
    fn safe_corrections_only_follow_unambiguous_issues() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        let text = "привт ,  HEllo";
        let report = checker.check(Some(&mut user), text, &settings).unwrap();
        // Safe auto-correction is off by default, so nothing is proposed.
        assert!(LocalSpellChecker::safe_corrections(&report, &settings).is_empty());
        // The double capital is reported as one whole word, not as `H` and `Ello`.
        assert!(
            report.issue_words().contains(&"HEllo"),
            "{:?}",
            report.issue_words()
        );

        let enabled = AutocorrectSettings {
            safe_autocorrect: true,
            ..AutocorrectSettings::default()
        };
        let corrections = LocalSpellChecker::safe_corrections(&report, &enabled);
        // The unknown word is never corrected; the space and the capital are.
        let originals: Vec<&str> = corrections
            .iter()
            .map(|correction| correction.original.as_str())
            .collect();
        assert!(originals.contains(&" "), "{originals:?}");
        assert!(originals.contains(&"HEllo"), "{originals:?}");
        assert!(!originals.contains(&"привт"));
        let capital = corrections
            .iter()
            .find(|correction| correction.original == "HEllo")
            .unwrap();
        assert_eq!(capital.replacement, "Hello");
        assert_eq!(capital.reason, Some(IssueReason::DoubleCapital));
    }

    #[test]
    fn a_check_without_the_user_dictionary_still_works_and_says_so() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        // The storage is locked: the encrypted words are unavailable, the dictionaries
        // are not, and the report says which of the two happened.
        let report = checker
            .check(None::<&mut TestUserDictionary>, "привт мир", &settings)
            .unwrap();
        assert!(!report.user_dictionary_available);
        assert_eq!(report.issue_words(), vec!["привт"]);
        assert!(report.issues[0]
            .suggestions
            .iter()
            .any(|suggestion| suggestion.text == "привет"));

        let with_user = checker
            .check(Some(&mut user), "привт мир", &settings)
            .unwrap();
        assert!(with_user.user_dictionary_available);
    }

    #[test]
    fn a_missing_dictionary_leaves_words_unverified_instead_of_wrong() {
        let directory = tempfile::tempdir().unwrap();
        let checker = LocalSpellChecker::from_manager(DictionaryManager::new(directory.path()));
        let settings = AutocorrectSettings::default();
        let report = checker
            .check(None::<&mut TestUserDictionary>, "привт мир", &settings)
            .unwrap();
        assert!(report.is_clean());
        assert_eq!(report.unavailable.len(), 2);
        assert!(!report.has_dictionary());
        assert_eq!(report.words_unverified, 2);
        assert_eq!(report.words_checked, 2);
    }

    #[test]
    fn the_user_dictionary_accepts_a_word_and_supplies_suggestions() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        user.add("Джарвис", Language::Russian, false).unwrap();

        let report = checker
            .check(Some(&mut user), "Джарвис и Джарвисс", &settings)
            .unwrap();
        // The stored word is accepted, differing only in case.
        assert_eq!(report.issue_words(), vec!["Джарвисс"]);
        let issue = &report.issues[0];
        assert!(issue.suggestions.iter().any(|suggestion| {
            suggestion.text == "Джарвис" && suggestion.source == SuggestionSource::UserDictionary
        }));
    }

    #[test]
    fn a_session_ignore_silences_a_word_without_storing_it() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        user.ignore_word("привт").unwrap();
        let report = checker
            .check(Some(&mut user), "привт мир", &settings)
            .unwrap();
        assert!(report.is_clean());
        assert!(user.is_ignored("ПРИВТ"));
        assert_eq!(user.stats().unwrap().words, 0);
        assert!(user.unignore_word("привт").unwrap());
        assert!(!checker
            .check(Some(&mut user), "привт", &settings)
            .unwrap()
            .is_clean());
    }

    #[test]
    fn a_user_rule_is_offered_and_applied_only_when_it_says_so() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings {
            custom_rules: vec![super::super::settings::CustomRule {
                pattern: "привт".to_string(),
                replacement: "привет".to_string(),
                auto_apply: false,
            }],
            safe_autocorrect: true,
            ..AutocorrectSettings::default()
        };
        let report = checker.check(Some(&mut user), "привт", &settings).unwrap();
        let issue = &report.issues[0];
        assert_eq!(issue.reason, IssueReason::UserRule);
        assert!(!issue.auto_fixable);
        assert_eq!(issue.suggestions[0].source, SuggestionSource::UserRule);
        assert!(LocalSpellChecker::safe_corrections(&report, &settings).is_empty());

        let auto = AutocorrectSettings {
            custom_rules: vec![super::super::settings::CustomRule {
                pattern: "привт".to_string(),
                replacement: "привет".to_string(),
                auto_apply: true,
            }],
            safe_autocorrect: true,
            ..AutocorrectSettings::default()
        };
        let report = checker.check(Some(&mut user), "привт", &auto).unwrap();
        let corrections = LocalSpellChecker::safe_corrections(&report, &auto);
        assert_eq!(corrections.len(), 1);
        assert_eq!(corrections[0].replacement, "привет");
    }

    #[test]
    fn the_yo_variant_is_accepted_as_correct_russian() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        // The fixture lists `ёж`, and `еж` is the same word.
        let report = checker
            .check(Some(&mut user), "еж и ёж", &settings)
            .unwrap();
        assert!(report.is_clean(), "{:?}", report.issue_words());
    }

    #[test]
    fn code_and_technical_tokens_are_left_alone() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        let text = "```\nlet speling = 1;\n```\nОткрой https://example.com/x и C:\\temp\\file.txt, id 550e8400-e29b-41d4-a716-446655440000, 42 штуки, API JARVIS";
        let report = checker.check(Some(&mut user), text, &settings).unwrap();
        assert!(report.is_clean(), "{:?}", report.issue_words());
        // The technical tokens were skipped rather than checked as words.
        assert!(report.words_skipped >= 5);
    }

    #[test]
    fn the_configured_limits_are_honoured() {
        use super::super::settings::MIN_MAX_ISSUES;
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings {
            max_issues: MIN_MAX_ISSUES,
            ..AutocorrectSettings::default()
        };
        let text = "слово1 ".repeat(40);
        let report = checker.check(Some(&mut user), &text, &settings).unwrap();
        assert_eq!(report.issues.len(), MIN_MAX_ISSUES);
        assert!(report.truncated);

        let strict = AutocorrectSettings {
            max_suggestions: 1,
            ..AutocorrectSettings::default()
        };
        let report = checker.check(Some(&mut user), "привт", &strict).unwrap();
        assert!(report.issues[0].suggestions.len() <= 1);
    }

    #[test]
    fn a_text_over_the_limit_is_refused_instead_of_truncated() {
        let (_directory, checker, mut user) = fixture();
        let checker = checker.with_max_chars(10);
        let settings = AutocorrectSettings::default();
        let error = checker
            .check(Some(&mut user), &"а".repeat(11), &settings)
            .unwrap_err();
        assert_eq!(error.code(), "text_too_large");
        assert!(checker
            .check(Some(&mut user), &"а".repeat(10), &settings)
            .is_ok());
    }

    #[test]
    fn a_switched_off_checker_reports_nothing_and_never_fails() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings {
            enabled: false,
            ..AutocorrectSettings::default()
        };
        let report = checker
            .check(Some(&mut user), "привт   ,  HEllo", &settings)
            .unwrap();
        assert!(!report.enabled);
        assert!(report.is_clean());
        assert_eq!(report.words_checked, 0);
    }

    #[test]
    fn the_check_reports_its_work_and_its_time() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        let report = checker
            .check(
                Some(&mut user),
                "привт мир, 42 и https://example.com/x",
                &settings,
            )
            .unwrap();
        assert_eq!(report.words_checked, 3);
        // The number and the URL carry no word to check.
        assert!(report.words_skipped >= 2);
        assert!(report.elapsed_ms < settings.timeout_ms);
        assert_eq!(report.issue_words(), vec!["привт"]);
    }

    #[test]
    fn a_suggestion_for_one_word_is_available_on_its_own() {
        let (_directory, checker, mut user) = fixture();
        let settings = AutocorrectSettings::default();
        let suggestions = checker
            .suggest(Some(&mut user), "привт", &settings)
            .unwrap();
        assert!(suggestions
            .iter()
            .any(|suggestion| suggestion.text == "привет"));
        assert!(checker
            .suggest(Some(&mut user), "42", &settings)
            .unwrap()
            .is_empty());
    }
}
