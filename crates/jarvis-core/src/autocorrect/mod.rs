//! Local autocorrect: checking, correcting, and explicitly improving text.
//!
//! ```text
//! text ──► tokenizer ──► LocalSpellChecker ──► CheckReport (issues + suggestions)
//!                            │        │
//!                            │        └── DictionaryManager ──► <dir>/ru_RU.aff|.dic
//!                            │                                          en_US.aff|.dic
//!                            └─────────── UserDictionaryStore ──► autocorrect.sqlite3
//!                                                    PurposeKeyProvider(JARVIS/autocorrect/v1)
//!
//! confirmed corrections ──► apply_corrections ──► CorrectionBatch ──► CorrectionJournal
//!                                                      │                     │
//!                                                      │                     └── undo_last
//!                                                      └── diff_texts ──► preview
//!
//! explicit "improve text" ──► LocalAiTextImprover ──► TextImprovementPreview ──► confirm
//!                            (the local gateway)        (never applied by itself)
//! ```
//!
//! Four properties hold across the whole feature:
//!
//! * **local**: spelling is checked against dictionaries on disk, the user's own word list
//!   is encrypted with a purpose-derived key, and the only model involved is the local
//!   `llama-server` the chat already uses. No text, word, or result leaves the machine;
//! * **nothing is applied silently**: a check reports, a correction is applied only when
//!   the user accepts it (safe auto-correction is off by default and can never touch an
//!   unknown word), and an AI improvement is applied only after its difference preview;
//! * **reversible**: every applied batch is journalled in memory and can be undone while
//!   the text is unchanged;
//! * **bounded**: text size, issue count, suggestion count, word count, check duration,
//!   and generation duration all have explicit limits.
//!
//! The password vault is deliberately outside this feature: autocorrect is never offered
//! on a vault page, and no vault record is scanned, corrected, or sent anywhere. Password
//! content is not prose, so a spell checker has nothing to say about it, and the safest
//! reading of "keep secrets out of the model" is not to feed them in.
//!
//! What is *not* here, on purpose: no cloud service, no dictionary shipped with the
//! application, no rewriting of the vault, no Voice/Whisper input, no Android, and no
//! network synchronization. See `docs/AUTOCORRECT.md`, `docs/ADR_AUTOCORRECT.md`, and
//! `docs/THREAT_MODEL_AUTOCORRECT.md`.

pub mod dictionary;
pub mod engine;
pub mod error;
pub mod improvement;
pub mod model;
pub mod replacement;
pub mod session;
pub mod settings;
pub mod tokenizer;
pub mod user_dictionary;

pub use dictionary::{
    DictionaryManager, DictionaryManifest, DictionaryManifestEntry, DictionaryState,
    LoadedDictionary, DICTIONARIES_DIR, DICTIONARY_MANIFEST_FILE,
};
pub use engine::{
    adapt_capitalisation, fix_double_capital, punctuation_issues, CheckReport, LocalSpellChecker,
};
pub use error::AutocorrectError;
pub use improvement::{
    apply_improvement, build_improvement_prompt, build_preview, check_improvement_input,
    preview_improvement, ImprovementPrompt, ImprovementWarning, LocalAiTextImprover,
    TextImprovementMode, TextImprovementPreview, TextImprovementProvider, TextImprovementRequest,
    IMPROVEMENT_TIMEOUT, MAX_IMPROVEMENT_TOKENS,
};
pub use model::{
    is_cyrillic, is_double_capital, normalize_word, now, text_version, yo_variant,
    AppliedCorrection, Correction, CorrectionBatch, CorrectionOutcome, IssueReason, Language,
    LanguageMode, SkippedCorrection, SpellingIssue, Suggestion, SuggestionSource, TextRange,
    UndoOutcome, UserDictionaryEntry, UserDictionaryPayload, Utf16Range, MAX_CHECK_CHARS,
    MAX_CUSTOM_INSTRUCTION_CHARS, MAX_DICTIONARY_WORD_CHARS, MAX_IGNORED_WORDS, MAX_IMPROVE_CHARS,
    MAX_ISSUES, MAX_SUGGESTIONS, MAX_TOKEN_CHARS, MAX_USER_DICTIONARY_ENTRIES,
};
pub use replacement::{
    apply_corrections, describe_diff, diff_texts, undo_last, CorrectionJournal, DiffKind,
    DiffSegment, TextDiff, MAX_JOURNAL_BATCHES,
};
pub use session::{
    database_has_records as autocorrect_database_has_records,
    database_path as autocorrect_database_path, dictionary_directory, dictionary_states,
    open_user_dictionary, resolved_dictionary_directory, unavailable_languages, AutocorrectStatus,
    AUTOCORRECT_DB_FILE,
};
pub use settings::{
    load_settings, AutocorrectSettings, CustomRule, SETTINGS_KEY as AUTOCORRECT_SETTINGS_KEY,
};
pub use tokenizer::{scan, scan_with_limit, ScanResult, SkipReason, SkippedToken, WordToken};
pub use user_dictionary::{
    bounded_distance, parse_word_list, DictionaryConflictView, EncryptedUserDictionary,
    ExportedDictionaryRecord, ImportOutcome, UserDictionaryQuery, UserDictionaryStats,
    UserDictionaryStore,
};
