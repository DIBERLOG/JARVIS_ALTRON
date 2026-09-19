//! Types of the local autocorrect feature: languages, ranges, issues, corrections,
//! the undo journal, and the text-improvement preview.
//!
//! # Text positions
//!
//! Two units appear here, and they are never mixed silently:
//!
//! * [`TextRange`] counts **Unicode scalar values** (Rust `char`s). Every range the
//!   engine produces is in this unit, because it is what this crate can slice safely;
//! * [`Utf16Range`] counts **UTF-16 code units**, which is what JavaScript string
//!   indices and `HTMLTextAreaElement.setSelectionRange` expect.
//!
//! A spelling issue carries both, computed together by [`TextRange::to_utf16`], so the
//! interface never has to guess and an emoji (two UTF-16 units, one char) cannot shift
//! a highlight. Byte offsets are never exposed.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::AutocorrectError;

/// Version of the JSON payload written into an encrypted user-dictionary record.
pub const USER_DICTIONARY_PAYLOAD_SCHEMA_VERSION: u32 = 1;

/// Most words one user dictionary may hold.
pub const MAX_USER_DICTIONARY_ENTRIES: usize = 20_000;
/// Longest word the user dictionary accepts, in characters.
pub const MAX_DICTIONARY_WORD_CHARS: usize = 64;
/// Longest text one interactive check accepts, in characters.
pub const MAX_CHECK_CHARS: usize = 200_000;
/// Most issues one check may report.
pub const MAX_ISSUES: usize = 500;
/// Hard cap for suggestions per issue.
pub const MAX_SUGGESTIONS: usize = 8;
/// Longest token the tokenizer will even look at.
pub const MAX_TOKEN_CHARS: usize = 40;
/// Most words the session ignore list may hold.
pub const MAX_IGNORED_WORDS: usize = 2_000;
/// Longest text the AI improvement accepts, in characters.
pub const MAX_IMPROVE_CHARS: usize = 20_000;
/// Longest custom improvement instruction, in characters.
pub const MAX_CUSTOM_INSTRUCTION_CHARS: usize = 500;
/// Most custom replacement rules a user may define.
pub const MAX_CUSTOM_RULES: usize = 200;

/// A language the checker knows about.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Russian,
    English,
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Russian => "russian",
            Self::English => "english",
        }
    }

    pub fn from_storage_name(value: &str) -> Result<Self, AutocorrectError> {
        match value {
            "russian" => Ok(Self::Russian),
            "english" => Ok(Self::English),
            _ => Err(AutocorrectError::InvalidConfiguration),
        }
    }

    pub fn all() -> [Self; 2] {
        [Self::Russian, Self::English]
    }

    /// File stem of the Hunspell dictionary pair, for example `ru_RU`.
    pub fn dictionary_stem(&self) -> &'static str {
        match self {
            Self::Russian => "ru_RU",
            Self::English => "en_US",
        }
    }

    /// File name of the affix file the user has to install.
    pub fn aff_file(&self) -> String {
        format!("{}.aff", self.dictionary_stem())
    }

    /// File name of the word-list file the user has to install.
    pub fn dic_file(&self) -> String {
        format!("{}.dic", self.dictionary_stem())
    }

    /// Which language a word is mostly written in, by its script.
    ///
    /// Cyrillic means Russian, Latin means English. A word with neither (digits,
    /// emoji, punctuation only) belongs to no language and is not checked.
    pub fn of_word(word: &str) -> Option<Self> {
        let mut cyrillic = 0usize;
        let mut latin = 0usize;
        for character in word.chars() {
            if character.is_alphabetic() {
                if is_cyrillic(character) {
                    cyrillic += 1;
                } else if character.is_ascii_alphabetic() {
                    latin += 1;
                }
            }
        }
        match (cyrillic, latin) {
            (0, 0) => None,
            (0, _) => Some(Self::English),
            (_, 0) => Some(Self::Russian),
            // A mixed word belongs to the script that dominates it.
            (cyrillic, latin) if cyrillic >= latin => Some(Self::Russian),
            _ => Some(Self::English),
        }
    }
}

/// Whether a character is in the Cyrillic block.
pub fn is_cyrillic(character: char) -> bool {
    matches!(character as u32,
        0x0400..=0x04FF   // Cyrillic
        | 0x0500..=0x052F // Cyrillic Supplement
        | 0x2DE0..=0x2DFF // Cyrillic Extended-A
        | 0xA640..=0xA69F // Cyrillic Extended-B
    )
}

/// Whether a word begins with two capitals followed by a lower-case letter.
///
/// That shape is a typo — `HEllo`, `ПРивет` — and it has exactly one sensible repair.
/// The third character is required to be lower case on purpose, which keeps identifiers
/// such as `HTTPServer` (checked as `HTTP` and `Server`) and names such as `McDonald`
/// out of this rule. An abbreviation never reaches it either, because the tokenizer
/// drops all-capitals tokens before any checker sees them.
pub fn is_double_capital(word: &str) -> bool {
    let characters: Vec<char> = word.chars().collect();
    if characters.len() < 3 {
        return false;
    }
    characters[0].is_uppercase() && characters[1].is_uppercase() && characters[2].is_lowercase()
}

/// Which dictionaries a check uses.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageMode {
    Russian,
    English,
    /// Decide per word, by script. This is the default: it is what a person writing in
    /// two languages expects, and it needs no setting changed.
    #[default]
    Auto,
    /// Check against both dictionaries; a word is correct if either knows it.
    Mixed,
}

impl LanguageMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Russian => "russian",
            Self::English => "english",
            Self::Auto => "auto",
            Self::Mixed => "mixed",
        }
    }

    pub fn from_storage_name(value: &str) -> Result<Self, AutocorrectError> {
        match value {
            "russian" => Ok(Self::Russian),
            "english" => Ok(Self::English),
            "auto" => Ok(Self::Auto),
            "mixed" => Ok(Self::Mixed),
            _ => Err(AutocorrectError::InvalidConfiguration),
        }
    }

    pub fn all() -> [Self; 4] {
        [Self::Russian, Self::English, Self::Auto, Self::Mixed]
    }

    /// Whether this mode consults more than one dictionary.
    pub fn is_mixed(&self) -> bool {
        matches!(self, Self::Mixed)
    }
}

/// A range in Unicode scalar values.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

impl TextRange {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// Whether two ranges share at least one position.
    pub fn overlaps(&self, other: &Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Converts to UTF-16 code units, which is what the interface slices with.
    ///
    /// `text` must be the text the range was computed on. A range that runs past the
    /// end is refused instead of being clamped, so a stale range can never select the
    /// wrong characters.
    pub fn to_utf16(&self, text: &str) -> Result<Utf16Range, AutocorrectError> {
        let total = text.chars().count();
        if self.end > total {
            return Err(AutocorrectError::RangeMismatch);
        }
        let mut utf16_start = None;
        let mut utf16_end = None;
        let mut utf16_units = 0usize;
        for (index, character) in text.chars().enumerate() {
            if index == self.start {
                utf16_start = Some(utf16_units);
            }
            if index == self.end {
                utf16_end = Some(utf16_units);
                break;
            }
            utf16_units += character.len_utf16();
        }
        let start = utf16_start.ok_or(AutocorrectError::RangeMismatch)?;
        let end = utf16_end.unwrap_or(utf16_units);
        Ok(Utf16Range { start, end })
    }

    /// The byte range in `text`, or `None` when the range is not on char boundaries.
    pub fn to_bytes(&self, text: &str) -> Option<std::ops::Range<usize>> {
        let mut start = None;
        let mut end = None;
        for (index, (offset, _)) in text.char_indices().enumerate() {
            if index == self.start {
                start = Some(offset);
            }
            if index == self.end {
                end = Some(offset);
                break;
            }
        }
        if self.end == text.chars().count() {
            end = Some(text.len());
        }
        Some(start?..end?)
    }

    /// Slices `text`, refusing a stale range.
    pub fn slice<'a>(&self, text: &'a str) -> Result<&'a str, AutocorrectError> {
        let bytes = self.to_bytes(text).ok_or(AutocorrectError::RangeMismatch)?;
        text.get(bytes).ok_or(AutocorrectError::RangeMismatch)
    }
}

/// A range in UTF-16 code units, as the interface uses it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Utf16Range {
    pub start: usize,
    pub end: usize,
}

/// Why a word was reported.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueReason {
    /// No dictionary and no user word knows it.
    UnknownWord,
    /// The word starts with two capitals, for example `HEllo`.
    DoubleCapital,
    /// Two or more spaces in a row.
    RepeatedSpace,
    /// A space before a comma, a period, and the like.
    SpaceBeforePunctuation,
    /// A user-defined replacement pair matched.
    UserRule,
    /// The whole text was replaced by an accepted AI improvement.
    ///
    /// It is never fixable automatically: an AI change is applied only after the user
    /// has seen the preview and confirmed it.
    AiImprovement,
}

impl IssueReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnknownWord => "unknown_word",
            Self::DoubleCapital => "double_capital",
            Self::RepeatedSpace => "repeated_space",
            Self::SpaceBeforePunctuation => "space_before_punctuation",
            Self::UserRule => "user_rule",
            Self::AiImprovement => "ai_improvement",
        }
    }

    /// Whether this reason may be applied without asking.
    ///
    /// Only rules with exactly one sensible answer are auto-fixable, and only when the
    /// user switched safe auto-correction on. An unknown word never is: a real name or
    /// term can be a valid word that the dictionary does not know, and an AI change is
    /// by definition a decision the user has to make.
    pub fn is_auto_fixable(&self) -> bool {
        matches!(
            self,
            Self::DoubleCapital
                | Self::RepeatedSpace
                | Self::SpaceBeforePunctuation
                | Self::UserRule
        )
    }
}

/// Where a suggestion came from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionSource {
    /// The Hunspell dictionary.
    Dictionary,
    /// The user's encrypted dictionary.
    UserDictionary,
    /// A user-defined replacement rule.
    UserRule,
    /// A mechanical text rule: a repeated space, a space before punctuation, or a
    /// word written with two capitals.
    TextRule,
    /// The same word with the other `ё`/`е` spelling.
    YoVariant,
}

/// One proposed replacement for a word.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Suggestion {
    pub text: String,
    pub source: SuggestionSource,
}

/// One problem found in the text.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpellingIssue {
    pub word: String,
    /// Range in Unicode scalar values.
    pub range: TextRange,
    /// The same range in UTF-16 code units, for the interface.
    pub utf16: Utf16Range,
    /// The language the word was checked against, when it could be decided.
    pub language: Option<Language>,
    pub suggestions: Vec<Suggestion>,
    pub reason: IssueReason,
    /// Whether safe auto-correction may apply this without asking.
    pub auto_fixable: bool,
}

impl SpellingIssue {
    /// The first suggestion, when there is one.
    pub fn best_suggestion(&self) -> Option<&str> {
        self.suggestions
            .first()
            .map(|suggestion| suggestion.text.as_str())
    }
}

/// A correction the interface asks for.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Correction {
    /// Range in Unicode scalar values, as reported by [`SpellingIssue`].
    pub range: TextRange,
    /// The text that must currently be at `range`; guards against stale ranges.
    pub original: String,
    pub replacement: String,
    /// Why the correction was proposed, when the interface knows. It is recorded in
    /// the undo journal and shown in the preview.
    #[serde(default)]
    pub reason: Option<IssueReason>,
}

/// What happened to one requested correction.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionOutcome {
    Applied,
    /// The text at the range was not the expected original.
    Mismatched,
    /// Another correction touched the same text.
    Overlapped,
}

/// One correction that was applied, kept for undo.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AppliedCorrection {
    pub id: Uuid,
    pub before: String,
    pub after: String,
    /// Where the replacement sits in the text **after** the batch was applied.
    pub range: TextRange,
    /// Hash of the document before and after the batch this correction belongs to.
    ///
    /// The version is per batch, not per correction, because undo reverts a whole
    /// batch by restoring the exact text that preceded it.
    pub version_before: String,
    pub version_after: String,
    pub applied_at: String,
    pub reason: IssueReason,
}

/// Result of applying a set of corrections.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CorrectionBatch {
    pub before: String,
    pub after: String,
    pub applied: Vec<AppliedCorrection>,
    pub skipped: Vec<SkippedCorrection>,
    pub version_before: String,
    pub version_after: String,
}

/// A correction that was refused, with the reason.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkippedCorrection {
    pub range: TextRange,
    pub original: String,
    pub outcome: CorrectionOutcome,
}

impl CorrectionBatch {
    pub fn applied_count(&self) -> usize {
        self.applied.len()
    }

    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }
}

/// Result of an undo.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UndoOutcome {
    /// The text with the last applied batch reverted, byte for byte.
    pub text: String,
    /// Every correction the batch had applied.
    pub restored: Vec<AppliedCorrection>,
    pub version_before: String,
    pub version_after: String,
    /// Applied batches left in the journal.
    pub remaining: usize,
}

/// One word in the encrypted user dictionary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UserDictionaryEntry {
    pub id: Uuid,
    pub revision: u64,
    pub word: String,
    pub language: Language,
    /// Whether the word was typed by the user or imported from a file.
    pub imported: bool,
    pub created_at: String,
}

/// The payload stored inside one encrypted dictionary record.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct UserDictionaryPayload {
    pub schema_version: u32,
    pub word: String,
    pub language: Language,
    pub imported: bool,
    pub created_at: String,
}

impl std::fmt::Debug for UserDictionaryPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserDictionaryPayload")
            .field("schema_version", &self.schema_version)
            .field("word", &"<redacted>")
            .field("language", &self.language)
            .field("imported", &self.imported)
            .finish()
    }
}

impl UserDictionaryPayload {
    pub fn create(
        word: &str,
        language: Language,
        imported: bool,
    ) -> Result<Self, AutocorrectError> {
        let payload = Self {
            schema_version: USER_DICTIONARY_PAYLOAD_SCHEMA_VERSION,
            word: normalize_word(word)?,
            language,
            imported,
            created_at: Utc::now().to_rfc3339(),
        };
        Ok(payload)
    }

    pub fn validate(&self) -> Result<(), AutocorrectError> {
        if self.schema_version != USER_DICTIONARY_PAYLOAD_SCHEMA_VERSION {
            return Err(AutocorrectError::InvalidConfiguration);
        }
        if normalize_word(&self.word)? != self.word {
            return Err(AutocorrectError::InvalidWord);
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, AutocorrectError> {
        serde_json::to_vec(self).map_err(|_| AutocorrectError::Io)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AutocorrectError> {
        let payload: Self = serde_json::from_slice(bytes).map_err(|_| AutocorrectError::Io)?;
        payload.validate()?;
        Ok(payload)
    }
}

/// Normalizes a word for storage and lookup.
///
/// A word is stored in lower case, with `ё` kept as the user wrote it: the two
/// spellings are compared as variants, but the stored form is the one the user
/// typed, so the dictionary does not silently rewrite their habit.
pub fn normalize_word(word: &str) -> Result<String, AutocorrectError> {
    let trimmed = word.trim();
    if trimmed.is_empty() {
        return Err(AutocorrectError::InvalidWord);
    }
    if trimmed.chars().count() > MAX_DICTIONARY_WORD_CHARS {
        return Err(AutocorrectError::InvalidWord);
    }
    if trimmed
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(AutocorrectError::InvalidWord);
    }
    if !trimmed.chars().any(char::is_alphabetic) {
        return Err(AutocorrectError::InvalidWord);
    }
    Ok(trimmed.to_lowercase())
}

/// The other spelling of a word with respect to `ё`/`е`.
///
/// Returns `None` when the word has neither, so a caller does not compare a word with
/// itself. Russian dictionaries vary in which spelling they list, and both are
/// accepted by the language, so the checker tries both.
pub fn yo_variant(word: &str) -> Option<String> {
    if word.contains('ё') {
        return Some(word.replace('ё', "е"));
    }
    if word.contains('Ё') {
        return Some(word.replace('Ё', "Е"));
    }
    if word.contains('е') {
        return Some(word.replace('е', "ё"));
    }
    if word.contains('Е') {
        return Some(word.replace('Е', "Ё"));
    }
    None
}

/// The current time, so every timestamp in this feature comes from one place.
pub fn now() -> chrono::DateTime<Utc> {
    Utc::now()
}

/// A stable hash of a text, used as its version.
///
/// The version is what makes a stale correction impossible: the interface sends the
/// version it checked, and applying a correction on anything else is refused.
pub fn text_version(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_map_to_their_dictionary_files() {
        assert_eq!(Language::Russian.dictionary_stem(), "ru_RU");
        assert_eq!(Language::English.dictionary_stem(), "en_US");
        assert_eq!(Language::Russian.aff_file(), "ru_RU.aff");
        assert_eq!(Language::English.dic_file(), "en_US.dic");
        for language in Language::all() {
            assert_eq!(
                Language::from_storage_name(language.as_str()).unwrap(),
                language
            );
        }
        assert!(Language::from_storage_name("german").is_err());
    }

    #[test]
    fn a_word_language_follows_its_script() {
        assert_eq!(Language::of_word("привет"), Some(Language::Russian));
        assert_eq!(Language::of_word("hello"), Some(Language::English));
        assert_eq!(Language::of_word("JARVIS"), Some(Language::English));
        // A mixed word belongs to the script that dominates it.
        assert_eq!(Language::of_word("ЯRVIS"), Some(Language::English));
        assert_eq!(Language::of_word("ЯЯRV"), Some(Language::Russian));
        assert_eq!(Language::of_word("12345"), None);
        assert_eq!(Language::of_word("—"), None);
        assert_eq!(Language::of_word("😀"), None);
        // A single Cyrillic letter is enough to make it Russian.
        assert_eq!(Language::of_word("ёж"), Some(Language::Russian));
    }

    #[test]
    fn every_language_mode_round_trips() {
        for mode in LanguageMode::all() {
            assert_eq!(
                LanguageMode::from_storage_name(mode.as_str()).unwrap(),
                mode
            );
        }
        assert!(LanguageMode::Mixed.is_mixed());
        assert!(!LanguageMode::Auto.is_mixed());
        assert!(LanguageMode::from_storage_name("klingon").is_err());
    }

    #[test]
    fn ranges_count_characters_and_convert_to_utf16() {
        // The emoji is one char and two UTF-16 units.
        let text = "😀 слво";
        let range = TextRange::new(2, 6);
        assert_eq!(range.slice(text).unwrap(), "слво");
        let utf16 = range.to_utf16(text).unwrap();
        assert_eq!(utf16.start, 3);
        assert_eq!(utf16.end, 7);
        assert_eq!(range.len(), 4);

        // A combining sequence is still two chars but one visible glyph.
        let combined = "e\u{0301}x";
        assert_eq!(combined.chars().count(), 3);
        assert_eq!(TextRange::new(0, 2).slice(combined).unwrap(), "e\u{0301}");
        assert_eq!(TextRange::new(0, 2).to_utf16(combined).unwrap().end, 2);

        // A stale range is refused rather than clamped.
        assert_eq!(
            TextRange::new(2, 40).to_utf16(text).unwrap_err(),
            AutocorrectError::RangeMismatch
        );
        assert_eq!(
            TextRange::new(0, 40).slice(text).unwrap_err(),
            AutocorrectError::RangeMismatch
        );
    }

    #[test]
    fn ranges_detect_overlap() {
        assert!(TextRange::new(0, 5).overlaps(&TextRange::new(3, 8)));
        assert!(!TextRange::new(0, 5).overlaps(&TextRange::new(5, 8)));
        assert!(TextRange::new(0, 5).overlaps(&TextRange::new(0, 5)));
        assert!(!TextRange::new(2, 3).is_empty());
        assert!(TextRange::new(3, 3).is_empty());
    }

    #[test]
    fn only_unambiguous_reasons_may_be_fixed_automatically() {
        assert!(IssueReason::DoubleCapital.is_auto_fixable());
        assert!(IssueReason::RepeatedSpace.is_auto_fixable());
        assert!(IssueReason::SpaceBeforePunctuation.is_auto_fixable());
        assert!(IssueReason::UserRule.is_auto_fixable());
        // An unknown word is never auto-fixed: a name may be a valid word.
        assert!(!IssueReason::UnknownWord.is_auto_fixable());
    }

    #[test]
    fn words_are_normalized_and_validated() {
        assert_eq!(normalize_word("  Привет  ").unwrap(), "привет");
        assert_eq!(normalize_word("JARVIS").unwrap(), "jarvis");
        assert_eq!(normalize_word("ёж").unwrap(), "ёж");
        assert_eq!(
            normalize_word("   ").unwrap_err(),
            AutocorrectError::InvalidWord
        );
        assert_eq!(
            normalize_word("два слова").unwrap_err(),
            AutocorrectError::InvalidWord
        );
        assert_eq!(
            normalize_word("12345").unwrap_err(),
            AutocorrectError::InvalidWord
        );
        assert_eq!(
            normalize_word(&"x".repeat(MAX_DICTIONARY_WORD_CHARS + 1)).unwrap_err(),
            AutocorrectError::InvalidWord
        );
        assert!(normalize_word(&"x".repeat(MAX_DICTIONARY_WORD_CHARS)).is_ok());
    }

    #[test]
    fn yo_variants_are_offered_for_both_spellings() {
        assert_eq!(yo_variant("ёж").as_deref(), Some("еж"));
        assert_eq!(yo_variant("еж").as_deref(), Some("ёж"));
        assert_eq!(yo_variant("ЁЖ").as_deref(), Some("ЕЖ"));
        assert_eq!(yo_variant("ЕЖ").as_deref(), Some("ЁЖ"));
        assert_eq!(yo_variant("hello"), None);
        assert_eq!(yo_variant(""), None);
        // The variant of a variant is the original.
        assert_eq!(
            yo_variant(&yo_variant("еж").unwrap()).as_deref(),
            Some("еж")
        );
    }

    #[test]
    fn a_text_version_changes_with_the_text() {
        let first = text_version("привет");
        assert_eq!(first, text_version("привет"));
        assert_ne!(first, text_version("привет!"));
        assert_ne!(first, text_version("Привет"));
        assert_eq!(first.len(), 16);
        assert!(first.chars().all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn a_dictionary_payload_round_trips_and_redacts_itself() {
        let payload = UserDictionaryPayload::create("Проект", Language::Russian, false).unwrap();
        assert_eq!(payload.word, "проект");
        let restored = UserDictionaryPayload::from_bytes(&payload.to_bytes().unwrap()).unwrap();
        assert_eq!(restored, payload);
        assert!(payload.validate().is_ok());

        let rendered = format!("{payload:?}");
        assert!(!rendered.contains("проект"));
        assert!(rendered.contains("<redacted>"));

        let mut broken = payload;
        broken.word = "Два слова".to_string();
        assert_eq!(
            broken.validate().unwrap_err(),
            AutocorrectError::InvalidWord
        );
    }
}
